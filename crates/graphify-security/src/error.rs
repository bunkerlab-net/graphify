//! Error type for URL/fetch/path validation and label sanitisation.

use std::net::IpAddr;
use std::path::PathBuf;

use thiserror::Error;

/// Errors from URL / fetch / path validation.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// URL scheme is not in the http/https allowlist.
    #[error("Blocked URL scheme '{scheme}' - only http and https are allowed. Got: '{url}'")]
    BlockedScheme {
        /// The blocked scheme, lowercased (e.g. `"file"`, `"ftp"`).
        scheme: String,
        /// The original URL.
        url: String,
    },

    /// Host matches a cloud metadata endpoint (e.g. `metadata.google.internal`).
    #[error("Blocked cloud metadata endpoint '{host}'. Got: '{url}'")]
    BlockedMetadataHost {
        /// The blocked host name.
        host: String,
        /// The original URL.
        url: String,
    },

    /// Resolved IP is in a private, loopback, or otherwise reserved range.
    #[error("Blocked private/internal IP {addr} (resolved from '{host}'). Got: '{url}'")]
    BlockedPrivateIp {
        /// The resolved address that was rejected.
        addr: IpAddr,
        /// The host name that resolved to `addr`.
        host: String,
        /// The original URL.
        url: String,
    },

    /// DNS resolution failed.
    #[error("DNS resolution failed for '{host}': {source}. Got: '{url}'")]
    DnsFailure {
        /// The host that failed to resolve.
        host: String,
        /// The original URL.
        url: String,
        /// The underlying I/O error from the resolver.
        #[source]
        source: std::io::Error,
    },

    /// The URL string did not parse.
    #[error("Could not parse URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    /// Response body exceeded the byte cap passed to `safe_fetch`.
    #[error("Response from '{url}' exceeds size limit ({mb} MB). Aborting download.")]
    SizeLimitExceeded {
        /// The URL whose response was too large.
        url: String,
        /// The byte cap expressed in MB (for the error message).
        mb: usize,
    },

    /// HTTP status code outside the 2xx range.
    #[error("HTTP error {status} from '{url}'")]
    HttpStatus {
        /// The URL that returned the error.
        url: String,
        /// The HTTP status code.
        status: u16,
    },

    /// Underlying I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Underlying HTTP transport error (DNS, TLS, connection reset, ...).
    #[error("Transport error: {0}")]
    Transport(String),

    /// `graphify-out/` base directory does not exist.
    #[error("Graph base directory does not exist: {0}. Run /graphify first to build the graph.")]
    BaseMissing(PathBuf),

    /// Resolved path escapes the `graphify-out/` base directory.
    #[error(
        "Path '{path}' escapes the allowed directory {base}. Only paths inside graphify-out/ are permitted."
    )]
    PathEscape {
        /// The path that would have escaped the base directory.
        path: PathBuf,
        /// The base directory the path was checked against.
        base: PathBuf,
    },

    /// The validated graph file does not exist.
    #[error("Graph file not found: {0}")]
    GraphFileMissing(PathBuf),

    /// URL parsed successfully but has no host (e.g. `http:///foo`).
    #[error("URL is missing a host. Got: '{0}'")]
    MissingHost(String),
}
