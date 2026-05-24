//! Helpers used only by `graphify-security`'s own integration tests so they
//! can hit a mockito server on `127.0.0.1`.
//!
//! Not part of the public API contract.

use std::time::Duration;

use crate::error::SecurityError;
use crate::fetch::fetch_with;

/// Like [`crate::safe_fetch`] but skips the private-IP rejection.
///
/// # Errors
///
/// Same set as [`crate::safe_fetch`] minus [`SecurityError::BlockedPrivateIp`].
pub fn fetch_allow_private(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, SecurityError> {
    fetch_with(url, max_bytes, timeout, true)
}

/// Like [`crate::safe_fetch_text`] but skips the private-IP rejection.
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
