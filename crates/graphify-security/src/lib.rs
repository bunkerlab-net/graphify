//! Security helpers — URL validation, safe fetch, path guards, label sanitisation.
//!
//! Ports `graphify-py/graphify/security.py`.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use ipnet::{Ipv4Net, Ipv6Net};
use regex::Regex;
use thiserror::Error;
use url::{Host, Url};

const ALLOWED_SCHEMES: &[&str] = &["http", "https"];

/// 50 MB hard cap for binary downloads.
pub const MAX_FETCH_BYTES: usize = 52_428_800;
/// 10 MB hard cap for HTML / text content.
pub const MAX_TEXT_BYTES: usize = 10_485_760;

const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.google.com"];

/// RFC 6598 Shared Address Space (CGN).
#[allow(clippy::expect_used)] // const-evaluated CIDR literal cannot fail at runtime
static CGN_NETWORK: LazyLock<Ipv4Net> =
    LazyLock::new(|| "100.64.0.0/10".parse().expect("static CGN CIDR literal"));
/// RFC 6052 NAT64 Well-Known Prefix.
#[allow(clippy::expect_used)] // const-evaluated CIDR literal cannot fail at runtime
static NAT64_WKP: LazyLock<Ipv6Net> =
    LazyLock::new(|| "64:ff9b::/96".parse().expect("static NAT64 CIDR literal"));

/// Errors from URL / fetch / path validation.
#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("Blocked URL scheme '{scheme}' - only http and https are allowed. Got: '{url}'")]
    BlockedScheme { scheme: String, url: String },

    #[error("Blocked cloud metadata endpoint '{host}'. Got: '{url}'")]
    BlockedMetadataHost { host: String, url: String },

    #[error("Blocked private/internal IP {addr} (resolved from '{host}'). Got: '{url}'")]
    BlockedPrivateIp {
        addr: IpAddr,
        host: String,
        url: String,
    },

    #[error("DNS resolution failed for '{host}': {source}. Got: '{url}'")]
    DnsFailure {
        host: String,
        url: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not parse URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("Response from '{url}' exceeds size limit ({mb} MB). Aborting download.")]
    SizeLimitExceeded { url: String, mb: usize },

    #[error("HTTP error {status} from '{url}'")]
    HttpStatus { url: String, status: u16 },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Graph base directory does not exist: {0}. Run /graphify first to build the graph.")]
    BaseMissing(PathBuf),

    #[error(
        "Path '{path}' escapes the allowed directory {base}. Only paths inside graphify-out/ are permitted."
    )]
    PathEscape { path: PathBuf, base: PathBuf },

    #[error("Graph file not found: {0}")]
    GraphFileMissing(PathBuf),
}

fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_blocked(v4),
        IpAddr::V6(v6) => {
            // If this is a NAT64 mapped IPv4, check the embedded v4 instead.
            if NAT64_WKP.contains(&v6) {
                let octets = v6.octets();
                let embedded = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
                return ipv4_is_blocked(embedded);
            }
            ipv6_is_blocked(v6)
        }
    }
}

fn ipv4_is_blocked(v4: Ipv4Addr) -> bool {
    if v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_multicast()
        || v4.is_unspecified()
    {
        return true;
    }
    if CGN_NETWORK.contains(&v4) {
        return true;
    }
    // 192.0.0.0/24 IETF protocol assignments, 198.18.0.0/15 benchmarking,
    // 240.0.0.0/4 reserved for future use — match Python's `is_reserved`.
    let o = v4.octets();
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return true;
    }
    if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
        return true;
    }
    if o[0] >= 240 {
        return true;
    }
    false
}

fn ipv6_is_blocked(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
        return true;
    }
    let segments = v6.segments();
    // Unique-local fc00::/7
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-local fe80::/10
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // IPv4-mapped ::ffff:0:0/96 — check the embedded v4
    if segments[0..5].iter().all(|s| *s == 0) && segments[5] == 0xffff {
        let v4 = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            (segments[6] & 0xff) as u8,
            (segments[7] >> 8) as u8,
            (segments[7] & 0xff) as u8,
        );
        return ipv4_is_blocked(v4);
    }
    // Documentation 2001:db8::/32
    if segments[0] == 0x2001 && segments[1] == 0xdb8 {
        return true;
    }
    false
}

/// Validate that `url` is http/https and does not resolve to a private/reserved IP.
///
/// Returns the original URL on success.
///
/// # Errors
///
/// Returns `SecurityError::BlockedScheme` for non-http(s) URLs,
/// `SecurityError::BlockedMetadataHost` for cloud metadata endpoints,
/// `SecurityError::BlockedPrivateIp` if DNS resolves to a private/reserved IP,
/// or `SecurityError::DnsFailure` if DNS resolution fails.
pub fn validate_url(url: &str) -> Result<String, SecurityError> {
    validate_url_with(url, false)
}

/// Inner validator. `allow_private_ips` is used only by inline tests that need
/// to talk to localhost mock HTTP servers.
fn validate_url_with(url: &str, allow_private_ips: bool) -> Result<String, SecurityError> {
    let parsed = Url::parse(url)?;
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(SecurityError::BlockedScheme {
            scheme,
            url: url.to_string(),
        });
    }
    let Some(host) = parsed.host() else {
        return Err(SecurityError::BlockedScheme {
            scheme,
            url: url.to_string(),
        });
    };

    let host_repr = match &host {
        Host::Domain(d) => d.to_string(),
        Host::Ipv4(a) => a.to_string(),
        Host::Ipv6(a) => a.to_string(),
    };
    let host_lower = host_repr.to_ascii_lowercase();
    if BLOCKED_HOSTS.contains(&host_lower.as_str()) {
        return Err(SecurityError::BlockedMetadataHost {
            host: host_repr,
            url: url.to_string(),
        });
    }

    match host {
        Host::Ipv4(addr) => {
            if !allow_private_ips && ip_is_blocked(IpAddr::V4(addr)) {
                return Err(SecurityError::BlockedPrivateIp {
                    addr: IpAddr::V4(addr),
                    host: host_repr,
                    url: url.to_string(),
                });
            }
        }
        Host::Ipv6(addr) => {
            if !allow_private_ips && ip_is_blocked(IpAddr::V6(addr)) {
                return Err(SecurityError::BlockedPrivateIp {
                    addr: IpAddr::V6(addr),
                    host: host_repr,
                    url: url.to_string(),
                });
            }
        }
        Host::Domain(domain) => {
            let port = parsed.port_or_known_default().unwrap_or(80);
            let lookup =
                (domain, port)
                    .to_socket_addrs()
                    .map_err(|e| SecurityError::DnsFailure {
                        host: domain.to_string(),
                        url: url.to_string(),
                        source: e,
                    })?;
            for sock in lookup {
                if !allow_private_ips && ip_is_blocked(sock.ip()) {
                    return Err(SecurityError::BlockedPrivateIp {
                        addr: sock.ip(),
                        host: domain.to_string(),
                        url: url.to_string(),
                    });
                }
            }
        }
    }

    Ok(url.to_string())
}

/// Fetch `url` and return raw bytes, with scheme + size protections applied.
///
/// # Errors
///
/// Returns any error from [`validate_url`], `SecurityError::HttpStatus` on
/// non-2xx, `SecurityError::SizeLimitExceeded` when the body exceeds
/// `max_bytes`, or `SecurityError::Transport`/`Io` on network failures.
pub fn safe_fetch(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, SecurityError> {
    fetch_with(url, max_bytes, timeout, false)
}

fn fetch_with(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
    allow_private_ips: bool,
) -> Result<Vec<u8>, SecurityError> {
    validate_url_with(url, allow_private_ips)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    fetch_inner(&agent, url, max_bytes, 10, allow_private_ips)
}

fn fetch_inner(
    agent: &ureq::Agent,
    url: &str,
    max_bytes: usize,
    redirects_left: usize,
    allow_private_ips: bool,
) -> Result<Vec<u8>, SecurityError> {
    let resp_result = agent
        .request("GET", url)
        .set("User-Agent", "Mozilla/5.0 graphify/1.0")
        .call();

    match resp_result {
        Ok(resp) => read_response(resp, url, max_bytes),
        Err(ureq::Error::Status(status, resp)) => {
            // Handle 3xx redirects ourselves so we can re-validate the target.
            if (300..400).contains(&status) {
                if redirects_left == 0 {
                    return Err(SecurityError::HttpStatus {
                        url: url.to_string(),
                        status,
                    });
                }
                if let Some(loc) = resp.header("Location") {
                    let next = match Url::parse(url).and_then(|base| base.join(loc)) {
                        Ok(u) => u.to_string(),
                        Err(e) => return Err(SecurityError::InvalidUrl(e)),
                    };
                    validate_url_with(&next, allow_private_ips)?;
                    return fetch_inner(
                        agent,
                        &next,
                        max_bytes,
                        redirects_left - 1,
                        allow_private_ips,
                    );
                }
                return Err(SecurityError::HttpStatus {
                    url: url.to_string(),
                    status,
                });
            }
            Err(SecurityError::HttpStatus {
                url: url.to_string(),
                status,
            })
        }
        Err(ureq::Error::Transport(t)) => Err(SecurityError::Transport(t.to_string())),
    }
}

fn read_response(
    resp: ureq::Response,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, SecurityError> {
    let mut reader = resp.into_reader().take((max_bytes as u64) + 1);
    let mut buf = Vec::new();
    std::io::copy(&mut reader, &mut buf)?;
    if buf.len() > max_bytes {
        return Err(SecurityError::SizeLimitExceeded {
            url: url.to_string(),
            mb: max_bytes / 1_048_576,
        });
    }
    Ok(buf)
}

/// Fetch `url` and return UTF-8 text (replacing bad bytes).
///
/// # Errors
///
/// Same set as [`safe_fetch`].
pub fn safe_fetch_text(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
) -> Result<String, SecurityError> {
    let raw = safe_fetch(url, max_bytes, timeout)?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

#[doc(hidden)]
pub mod test_support {
    //! Helpers used only by `graphify-security`'s own integration tests so
    //! they can hit a mockito server on `127.0.0.1`. Not part of the public
    //! API contract.
    use std::time::Duration;

    use super::SecurityError;

    /// Like [`super::safe_fetch`] but skips private-IP rejection.
    ///
    /// # Errors
    ///
    /// Same set as [`super::safe_fetch`] minus `BlockedPrivateIp`.
    pub fn fetch_allow_private(
        url: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, SecurityError> {
        super::fetch_with(url, max_bytes, timeout, true)
    }

    /// Like [`super::safe_fetch_text`] but skips private-IP rejection.
    ///
    /// # Errors
    ///
    /// Same set as [`fetch_allow_private`].
    pub fn fetch_text_allow_private(
        url: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> Result<String, SecurityError> {
        let raw = fetch_allow_private(url, max_bytes, timeout)?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }
}

/// Resolve `path` and verify it stays inside `base`. `base` defaults to the
/// `graphify-out/` directory relative to CWD.
///
/// # Errors
///
/// Returns `SecurityError::BaseMissing` if the base directory does not exist,
/// `SecurityError::PathEscape` if the path resolves outside the base, or
/// `SecurityError::GraphFileMissing` if the file itself does not exist.
pub fn validate_graph_path<P: AsRef<Path>>(
    path: P,
    base: Option<&Path>,
) -> Result<PathBuf, SecurityError> {
    let path = path.as_ref();

    let base_path = if let Some(b) = base {
        b.to_path_buf()
    } else {
        // Walk up from the hint path looking for a "graphify-out" parent. Falls
        // back to CWD/graphify-out, matching the Python behaviour.
        let hint = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut found: Option<PathBuf> = None;
        let mut cur = Some(hint.as_path());
        while let Some(c) = cur {
            if c.file_name().is_some_and(|n| n == "graphify-out") {
                found = Some(c.to_path_buf());
                break;
            }
            cur = c.parent();
        }
        found.unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| PathBuf::from("graphify-out"),
                |cwd| cwd.join("graphify-out"),
            )
        })
    };

    let Ok(base_resolved) = base_path.canonicalize() else {
        return Err(SecurityError::BaseMissing(base_path));
    };
    if !base_resolved.exists() {
        return Err(SecurityError::BaseMissing(base_resolved));
    }

    // For path resolution we want to mirror Python's `Path.resolve()` which
    // resolves parent components even when the path doesn't exist. We do this
    // by canonicalizing the parent and joining the file name.
    let resolved = resolve_logical(path);

    if resolved.strip_prefix(&base_resolved).is_err() {
        return Err(SecurityError::PathEscape {
            path: path.to_path_buf(),
            base: base_resolved,
        });
    }

    if !resolved.exists() {
        return Err(SecurityError::GraphFileMissing(resolved));
    }

    Ok(resolved)
}

fn resolve_logical(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for comp in absolute.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR.to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
        }
    }
    // Canonicalize the deepest existing prefix so symlink-resolution matches
    // Python's Path.resolve for the existing portion.
    let mut existing: Option<PathBuf> = None;
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let components: Vec<_> = out.components().collect();
    for (idx, _) in components.iter().enumerate().rev() {
        let candidate: PathBuf = components[..=idx].iter().collect();
        if candidate.exists() {
            existing = Some(candidate);
            for c in &components[idx + 1..] {
                if let Component::Normal(name) = c {
                    tail.push(name);
                }
            }
            break;
        }
    }
    if let Some(ex) = existing
        && let Ok(canon) = ex.canonicalize()
    {
        let mut result = canon;
        for t in tail {
            result.push(t);
        }
        return result;
    }
    out
}

#[allow(clippy::expect_used)] // static regex pattern is a literal; cannot fail at runtime
static CONTROL_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f]").expect("static control-char regex"));

/// Strip control characters and cap length at 256.
///
/// Safe for embedding in JSON inside `<script>` tags. For direct HTML
/// injection, wrap the result with [`htmlescape::encode_minimal`].
#[must_use]
pub fn sanitize_label(text: Option<&str>) -> String {
    let Some(text) = text else {
        return String::new();
    };
    let cleaned = CONTROL_CHARS.replace_all(text, "");
    if cleaned.chars().count() > MAX_LABEL_LEN {
        cleaned.chars().take(MAX_LABEL_LEN).collect()
    } else {
        cleaned.into_owned()
    }
}

const MAX_LABEL_LEN: usize = 256;
