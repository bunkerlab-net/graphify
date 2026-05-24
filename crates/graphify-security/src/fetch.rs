//! Safe HTTP fetch with scheme + size protections and SSRF-checked
//! redirects.

use std::time::Duration;

use url::Url;

use crate::error::SecurityError;
use crate::url_guard::validate_url_with;

/// Hard cap for binary downloads passed to [`safe_fetch`]: 50 MiB (52,428,800 bytes).
///
/// Callers that need a lower limit should pass a smaller value directly;
/// this constant is the absolute ceiling for general-purpose binary fetches.
pub const MAX_FETCH_BYTES: usize = 52_428_800;

/// Hard cap for HTML / text content passed to [`safe_fetch_text`]: 10 MiB (10,485,760 bytes).
///
/// Text responses larger than this are rejected with
/// [`SecurityError::SizeLimitExceeded`] to avoid memory exhaustion when
/// processing untrusted web content.
pub const MAX_TEXT_BYTES: usize = 10_485_760;

/// Fetch `url` and return the raw body, applying:
/// - scheme allowlist + cloud-metadata blocklist + SSRF check via
///   [`crate::validate_url`],
/// - a hard cap of `max_bytes`,
/// - manual redirect handling so every hop is re-validated.
///
/// # Errors
///
/// Returns any error from [`crate::validate_url`],
/// [`SecurityError::HttpStatus`] on non-2xx, [`SecurityError::SizeLimitExceeded`]
/// when the body exceeds `max_bytes`, or [`SecurityError::Transport`] /
/// [`SecurityError::Io`] on network failures.
pub fn safe_fetch(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, SecurityError> {
    // Test-only env-var bypass — see `validate_url` in `url_guard.rs`.
    let allow_private = std::env::var("GRAPHIFY_TEST_ALLOW_PRIVATE_IPS")
        .ok()
        .is_some_and(|v| v == "1");
    fetch_with(url, max_bytes, timeout, allow_private)
}

/// Like [`safe_fetch`] but exposes the `allow_private_ips` knob used by the
/// test-support harness.
pub(crate) fn fetch_with(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
    allow_private_ips: bool,
) -> Result<Vec<u8>, SecurityError> {
    validate_url_with(url, allow_private_ips)?;
    // ureq 3 redesigned the agent builder. We disable auto-redirects so we
    // can re-validate the target URL on every hop, and disable the implicit
    // status-as-error wrapping so 3xx responses return as `Ok(response)`
    // (we need access to the `Location` header on those).
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    fetch_inner(&agent, url, max_bytes, 10, allow_private_ips)
}

/// Execute a single GET, re-validating the target on every redirect.
fn fetch_inner(
    agent: &ureq::Agent,
    url: &str,
    max_bytes: usize,
    redirects_left: usize,
    allow_private_ips: bool,
) -> Result<Vec<u8>, SecurityError> {
    let resp_result = agent
        .get(url)
        .header("User-Agent", "Mozilla/5.0 graphify/1.0")
        .call();

    let resp = match resp_result {
        Ok(r) => r,
        Err(e) => return Err(SecurityError::Transport(e.to_string())),
    };

    let status = resp.status().as_u16();

    if (300..400).contains(&status) {
        if redirects_left == 0 {
            return Err(SecurityError::HttpStatus {
                url: url.to_string(),
                status,
            });
        }
        let location = resp
            .headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if let Some(loc) = location {
            let next = match Url::parse(url).and_then(|base| base.join(&loc)) {
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

    if !(200..300).contains(&status) {
        return Err(SecurityError::HttpStatus {
            url: url.to_string(),
            status,
        });
    }

    read_response(resp, url, max_bytes)
}

/// Read a 2xx response body up to `max_bytes`, returning
/// [`SecurityError::SizeLimitExceeded`] if the body exceeds the cap.
fn read_response(
    resp: ureq::http::Response<ureq::Body>,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, SecurityError> {
    // ureq 3's `Body::with_config().limit(N).read_to_vec()` returns
    // `Err(BodyExceedsLimit)` when the response exceeds N bytes. We set
    // the limit one byte higher than `max_bytes` so we can distinguish
    // "fits exactly" from "exceeded" via the error path.
    let limit = (max_bytes as u64) + 1;
    match resp.into_body().with_config().limit(limit).read_to_vec() {
        Ok(buf) if buf.len() > max_bytes => Err(SecurityError::SizeLimitExceeded {
            url: url.to_string(),
            mb: max_bytes / 1_048_576,
        }),
        Ok(buf) => Ok(buf),
        Err(ureq::Error::BodyExceedsLimit(_)) => Err(SecurityError::SizeLimitExceeded {
            url: url.to_string(),
            mb: max_bytes / 1_048_576,
        }),
        Err(e) => Err(SecurityError::Transport(e.to_string())),
    }
}

/// Fetch `url` and return the body decoded as UTF-8 (lossy: bad bytes are
/// replaced with U+FFFD).
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
