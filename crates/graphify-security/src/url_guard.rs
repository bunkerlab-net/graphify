//! URL validation: scheme allowlist, cloud-metadata-host blocklist, and
//! SSRF-protective resolved-IP check.

use std::net::{IpAddr, ToSocketAddrs};

use url::{Host, Url};

use crate::error::SecurityError;
use crate::ip::ip_is_blocked;

const ALLOWED_SCHEMES: &[&str] = &["http", "https"];

const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.google.com"];

/// Validate that `url` is http/https and does not resolve to a
/// private/reserved IP.
///
/// On success returns the original URL string unchanged so the caller can
/// chain `safe_fetch(validate_url(url)?)` idiomatically.
///
/// # Errors
///
/// Returns:
/// - [`SecurityError::BlockedScheme`] for non-http(s) URLs.
/// - [`SecurityError::BlockedMetadataHost`] for cloud metadata endpoints.
/// - [`SecurityError::BlockedPrivateIp`] if DNS resolves to a private or
///   reserved IP.
/// - [`SecurityError::DnsFailure`] if DNS resolution fails.
pub fn validate_url(url: &str) -> Result<String, SecurityError> {
    // Test-only escape hatch: when `GRAPHIFY_TEST_ALLOW_PRIVATE_IPS=1` is set,
    // skip the private-IP block so mockito-driven tests can hit 127.0.0.1.
    // The env var name is deliberately verbose to avoid accidental enablement
    // and never appears in production deployment guidance.
    let allow_private = std::env::var("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS")
        .ok()
        .is_some_and(|v| v == "1");
    validate_url_with(url, allow_private)
}

/// Like [`validate_url`] but optionally bypasses private-IP rejection.
///
/// Used by the test-support harness to permit hits against mockito on
/// `127.0.0.1`.
pub(crate) fn validate_url_with(
    url: &str,
    allow_private_ips: bool,
) -> Result<String, SecurityError> {
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
