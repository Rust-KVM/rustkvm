use std::net::IpAddr;

use salvo::conn::rustls::{Keycert, RustlsConfig};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::config::get_config_manager;

pub async fn init() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Ok(())
}

pub async fn init_rustls_config(local_ip: Option<IpAddr>) -> anyhow::Result<RustlsConfig> {
    let mut subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    if let Some(local_ip) = local_ip {
        subject_alt_names.push(local_ip.to_string());
    }
    let cert = rcgen::generate_simple_self_signed(subject_alt_names)
        .map_err(|e| anyhow::anyhow!("failed to generate self-signed certificate: {e}"))?;
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();
    let key_cert = Keycert::new().cert(cert_pem).key(key_pem);
    Ok(RustlsConfig::new(key_cert))
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TlsState {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "privateKey")]
    pub private_key: Option<String>,
}

pub async fn get_tls_state() -> anyhow::Result<TlsState> {
    let mgr = get_config_manager();
    let cfg = mgr.get().await;
    let mode =
        if cfg.tls_mode == "custom" { "custom".to_string() } else { "self-signed".to_string() };
    Ok(TlsState { mode, certificate: None, private_key: None })
}

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

    let mgr = get_config_manager();
    if mode == "custom"
        && (_certificate_pem.unwrap_or("").is_empty() || _private_key_pem.unwrap_or("").is_empty())
    {
        anyhow::bail!("certificate and privateKey are required for custom mode");
    }

    let mode_to_save =
        if mode == "custom" { "custom".to_string() } else { "self-signed".to_string() };
    mgr.update(move |cfg| {
        cfg.tls_mode = mode_to_save;
    })
    .await?;

    info!("TLS mode updated: {}", mode);
    Ok(())
}
