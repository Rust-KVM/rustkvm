use std::net::IpAddr;

use salvo::conn::rustls::{Keycert, RustlsConfig};
// use axum_server::tls_rustls::RustlsConfig;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::config::get_config_manager;

pub async fn init() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default provider");
    Ok(())
}

/// Initializes and returns a RustlsConfig with a self-signed certificate
///
/// # Returns
/// * `RustlsConfig` - The TLS configuration with self-signed certificate
///
/// # Implementation Details
/// - Creates a list of subject alternative names including:
///   - localhost
///   - 127.0.0.1
///   - Local IP address (if available)
/// - Generates a self-signed certificate with these names
/// - Extracts PEM-encoded certificate and private key
/// - Creates RustlsConfig from the certificate and key
///
/// # Errors
/// Will panic if:
/// - Certificate generation fails
/// - Converting PEM data to RustlsConfig fails
///
/// # Example
/// ```
/// let config = init_rustls_config(local_ip).await;
/// let server = axum_server::bind_rustls(addr, config);
/// ```
pub async fn init_rustls_config(local_ip: Option<IpAddr>) -> anyhow::Result<RustlsConfig> {
    let mut subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    if let Some(local_ip) = local_ip {
        subject_alt_names.push(local_ip.to_string());
    }
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .expect("Failed to generate self-signed certificate");
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();
    let key_cert = Keycert::new().cert(cert_pem).key(key_pem);
    Ok(RustlsConfig::new(key_cert))
}

/// TLS state
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TlsState {
    pub mode: String, // "disabled" | "custom" | "self-signed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "privateKey")]
    pub private_key: Option<String>,
}

/// Get current TLS state from configuration
pub async fn get_tls_state() -> anyhow::Result<TlsState> {
    let mgr = get_config_manager();
    let cfg = mgr.get().await;
    let mode =
        if cfg.tls_mode == "custom" { "custom".to_string() } else { "self-signed".to_string() };
    Ok(TlsState { mode, certificate: None, private_key: None })
}

/// Apply TLS state and persist configuration
/// For now we only persist the mode. Custom certificate storage can be added later.
pub async fn apply_tls_state(
    mode: &str,
    _certificate_pem: Option<&str>,
    _private_key_pem: Option<&str>,
) -> anyhow::Result<()> {
    match mode {
        "disabled" => anyhow::bail!("TLS disabled is not supported"),
        "custom" | "self-signed" => {}
        other => anyhow::bail!(format!("invalid TLS mode: {}", other)),
    }

    // Persist mode to config
    let mgr = get_config_manager();
    if mode == "custom"
        && (_certificate_pem.unwrap_or("").is_empty() || _private_key_pem.unwrap_or("").is_empty())
    {
        anyhow::bail!("certificate and privateKey are required for custom mode");
    }
    // TODO: persist PEM and apply to live server if hot-reload is supported

    let mode_to_save =
        if mode == "custom" { "custom".to_string() } else { "self-signed".to_string() };
    mgr.update(move |cfg| {
        cfg.tls_mode = mode_to_save;
    })
    .await?;

    // Hook for dynamic server control can be added here if needed
    info!("TLS mode updated: {}", mode);
    Ok(())
}
