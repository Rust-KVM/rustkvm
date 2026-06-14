use thiserror::Error;

/// Errors produced by `rkvm_net`.
///
/// Library APIs return this enum directly so callers can `match` on a specific
/// failure mode instead of string-matching an `anyhow` chain. The app layer keeps
/// using `anyhow` and absorbs these via `?` (thiserror implements `std::error::Error`).
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    // `macaddr::ParseError` does not implement `std::error::Error` under the
    // crate's `default-features = false` build, so we capture its `Display` text
    // rather than chaining it as a `#[source]`.
    #[error("invalid MAC address `{mac}`: {reason}")]
    InvalidMac { mac: String, reason: String },

    #[error("failed to execute `{cmd}`")]
    CommandExec {
        cmd: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON output: {0}")]
    Json(#[from] serde_json::Error),

    #[error("control URL must start with http:// or https://")]
    InvalidControlUrl,

    #[error("mDNS server already running")]
    MdnsAlreadyRunning,

    #[error("mDNS error: {0}")]
    Mdns(#[from] mdns_sd::Error),
}

/// Convenience alias for `Result<T, rkvm_net::Error>`.
pub type Result<T> = std::result::Result<T, Error>;
