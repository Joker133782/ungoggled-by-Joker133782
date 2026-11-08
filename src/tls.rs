//! A self-signed TLS certificate for the web interface, generated once per
//! device and kept in the data directory from then on.
//!
//! A real, publicly trusted certificate (Let's Encrypt or similar) needs a
//! publicly resolvable domain name, which a private Wi-Fi access point at a
//! fixed local IP does not have, so that is not an option here. A
//! self-signed certificate still unlocks something real: without HTTPS at
//! all, Chrome and other browsers refuse to treat the page as installable —
//! "Add to Home Screen" falls back to a plain bookmark that still shows the
//! address bar. The browser will show a one-time warning the first time it
//! sees this certificate ("Your connection is not private"); clicking
//! through it ("Advanced" → "Proceed") is still required once per device,
//! but after that the connection is a genuine secure context and the page
//! can be installed as a real fullscreen app.
use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use std::{fs, path::Path};

/// Load the certificate from `dir`, generating and persisting a new one if
/// none exists yet.
pub async fn ensure(dir: &Path) -> Result<RustlsConfig> {
    // Idempotent: harmless if a previous call (or test) already installed it.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert_path = dir.join("tls-cert.pem");
    let key_path = dir.join("tls-key.pem");
    let (cert_pem, key_pem) = match (fs::read(&cert_path), fs::read(&key_path)) {
        (Ok(cert_pem), Ok(key_pem)) => (cert_pem, key_pem),
        _ => {
            // The access point's own fixed address, so the certificate
            // actually names what a browser will connect to; localhost
            // covers testing against the server directly on the device.
            let names = ["192.168.50.1", "127.0.0.1", "localhost"]
                .map(String::from)
                .to_vec();
            let CertifiedKey { cert, key_pair } = generate_simple_self_signed(names)
                .context("Cannot generate the self-signed TLS certificate")?;
            let cert_pem = cert.pem().into_bytes();
            let key_pem = key_pair.serialize_pem().into_bytes();
            crate::settings::atomic_write(&cert_path, &cert_pem)?;
            crate::settings::atomic_write(&key_path, &key_pem)?;
            (cert_pem, key_pem)
        }
    };
    RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .context("Cannot load the TLS certificate")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn generates_once_and_reuses_the_same_certificate() {
        let dir = std::env::temp_dir().join(format!(
            "ungoggled-tls-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        ensure(&dir).await.unwrap();
        let cert = fs::read(dir.join("tls-cert.pem")).unwrap();
        let key = fs::read(dir.join("tls-key.pem")).unwrap();
        assert!(!cert.is_empty());
        assert!(!key.is_empty());
        // A second call must not regenerate (and thus replace) the pair.
        ensure(&dir).await.unwrap();
        assert_eq!(fs::read(dir.join("tls-cert.pem")).unwrap(), cert);
        assert_eq!(fs::read(dir.join("tls-key.pem")).unwrap(), key);
        fs::remove_dir_all(&dir).unwrap();
    }
}
