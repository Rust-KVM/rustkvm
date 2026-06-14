use thiserror::Error;

/// Errors produced by `rkvm_core`.
///
/// Library APIs return this enum directly so callers can `match` on specific
/// failure modes (e.g. retry on `Http`, fall back on `AllSourcesFailed`) without
/// resorting to string comparison on an `anyhow::Error` chain.
#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("response missing Date header")]
    MissingDateHeader,

    #[error("Date header contained non-ASCII bytes: {0}")]
    InvalidDateHeader(#[source] reqwest::header::ToStrError),

    #[error("failed to parse HTTP date `{date}`")]
    ParseDate {
        date: String,
        #[source]
        source: httpdate::Error,
    },

    #[error("system time is before the UNIX epoch")]
    TimeBeforeEpoch(#[from] std::time::SystemTimeError),

    #[error("clock_settime(2) failed")]
    ClockSettime(#[source] std::io::Error),

    #[error("all HTTP time sources failed")]
    AllSourcesFailed,
}

/// Convenience alias for `Result<T, rkvm_core::Error>`.
pub type Result<T> = std::result::Result<T, Error>;
