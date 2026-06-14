pub const BUILT_APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn built_app_version() -> &'static str {
    BUILT_APP_VERSION
}

pub struct VersionInfo {
    pub version: &'static str,
    pub revision: &'static str,
    pub branch: &'static str,
    pub build_date: &'static str,
    pub rust_version: &'static str,
    pub platform: String,
}

impl VersionInfo {
    pub fn current() -> Self {
        Self {
            version: BUILT_APP_VERSION,
            revision: option_env!("GIT_REVISION").unwrap_or("unknown"),
            branch: option_env!("GIT_BRANCH").unwrap_or("unknown"),
            build_date: option_env!("BUILD_DATE").unwrap_or("unknown"),
            rust_version: option_env!("RUSTC_VERSION").unwrap_or("unknown"),
            platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        }
    }
}
