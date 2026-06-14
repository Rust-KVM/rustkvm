use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

pub(super) async fn build_tls_config(insecure: bool) -> Result<Arc<rustls::ClientConfig>> {
    if insecure {
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();
        Ok(Arc::new(config))
    } else {
        let mut root_store = rustls::RootCertStore::empty();
        load_native_certs(&mut root_store).await;
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(Arc::new(config))
    }
}

fn parse_pem_certs(data: &[u8]) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    use base64::Engine as _;
    let mut certs = Vec::new();
    let mut in_cert = false;
    let mut b64 = String::new();

    for line in data.split(|&b| b == b'\n') {
        let line = std::str::from_utf8(line).unwrap_or("");
        if line.starts_with("-----BEGIN CERTIFICATE-----") {
            in_cert = true;
            b64.clear();
        } else if line.starts_with("-----END CERTIFICATE-----") {
            if in_cert {
                in_cert = false;
                if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                    certs.push(rustls::pki_types::CertificateDer::from(der));
                }
            }
        } else if in_cert {
            b64.push_str(line.trim());
        }
    }
    certs
}

async fn load_native_certs(root_store: &mut rustls::RootCertStore) {
    let cert_paths = [
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/ssl/certs/ca-bundle.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/ssl/ca-bundle.pem",
    ];

    for path in &cert_paths {
        if let Ok(data) = tokio::fs::read(path).await {
            let certs = parse_pem_certs(&data);
            let count = certs.len();
            for cert in certs {
                let _ = root_store.add(cert);
            }
            if count > 0 {
                info!("Loaded {} CA certificates from {}", count, path);
                return;
            }
        }
    }

    warn!(
        "No system CA certificates found — TLS connections may fail unless tls_insecure is enabled"
    );
}

#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
