//! Outbound HTTPS trust settings: rippled `[ssl_verify]`, `[ssl_verify_file]`
//! and `[ssl_verify_dir]` (`HTTPClientSSLContext`).
//!
//! rippled semantics:
//! - no `ssl_verify_file`: trust the platform default roots (plus the dir);
//! - `ssl_verify_file`: trust only the certificates in that file (plus the
//!   dir); the default roots are not loaded;
//! - `ssl_verify_dir`: additionally trust every certificate in the directory;
//!   an unreadable directory is a startup error;
//! - `ssl_verify` (default 1): verify the server certificate and host name.
//!
//! The settings are configured once at startup and applied to every outbound
//! HTTPS client (validator list sites and the work/HTTP clients).

use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundTls {
    /// Verify the server certificate and host name.
    pub verify: bool,
    /// Trust the built-in (default) roots.
    pub builtin_roots: bool,
    /// Extra trusted roots (DER).
    pub extra_roots: Vec<Vec<u8>>,
}

impl Default for OutboundTls {
    fn default() -> Self {
        Self {
            verify: true,
            builtin_roots: true,
            extra_roots: Vec::new(),
        }
    }
}

static OUTBOUND_TLS: OnceLock<OutboundTls> = OnceLock::new();

/// Build the trust settings from the three config values.
pub fn outbound_tls_from(
    ssl_verify_dir: &str,
    ssl_verify_file: &str,
    ssl_verify: bool,
) -> Result<OutboundTls, String> {
    let mut tls = OutboundTls {
        verify: ssl_verify,
        builtin_roots: ssl_verify_file.is_empty(),
        extra_roots: Vec::new(),
    };
    if !ssl_verify_file.is_empty() {
        tls.extra_roots.extend(
            read_pem_certs(Path::new(ssl_verify_file))
                .map_err(|e| format!("Failed to load ssl_verify_file '{ssl_verify_file}': {e}"))?,
        );
    }
    if !ssl_verify_dir.is_empty() {
        let entries = fs::read_dir(ssl_verify_dir)
            .map_err(|e| format!("Failed to add verify path: {e}"))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Ok(certs) = read_pem_certs(&path)
            {
                tls.extra_roots.extend(certs);
            }
        }
    }
    Ok(tls)
}

fn read_pem_certs(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(certs.into_iter().map(|cert| cert.as_ref().to_vec()).collect())
}

/// Install the process-wide settings (first call wins).
pub fn configure_outbound_tls(tls: OutboundTls) {
    let _ = OUTBOUND_TLS.set(tls);
}

/// The active settings (rippled defaults until configured).
pub fn outbound_tls() -> OutboundTls {
    OUTBOUND_TLS.get().cloned().unwrap_or_default()
}

/// Apply the settings to an async reqwest client.
pub fn apply_outbound_tls(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let tls = outbound_tls();
    let mut builder = builder
        .use_rustls_tls()
        .tls_built_in_root_certs(tls.builtin_roots)
        .danger_accept_invalid_certs(!tls.verify);
    for der in &tls.extra_roots {
        if let Ok(cert) = reqwest::Certificate::from_der(der) {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder
}

/// Apply the settings to a blocking reqwest client.
pub fn apply_outbound_tls_blocking(
    builder: reqwest::blocking::ClientBuilder,
) -> reqwest::blocking::ClientBuilder {
    let tls = outbound_tls();
    let mut builder = builder
        .use_rustls_tls()
        .tls_built_in_root_certs(tls.builtin_roots)
        .danger_accept_invalid_certs(!tls.verify);
    for der in &tls.extra_roots {
        if let Ok(cert) = reqwest::Certificate::from_der(der) {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_verify_with_builtin_roots() {
        let tls = outbound_tls_from("", "", true).expect("defaults");
        assert_eq!(tls, OutboundTls::default());
    }

    #[test]
    fn ssl_verify_zero_disables_verification() {
        assert!(!outbound_tls_from("", "", false).expect("no verify").verify);
    }

    #[test]
    fn verify_file_replaces_builtin_roots_and_missing_file_fails() {
        assert!(outbound_tls_from("", "/nonexistent/ca.pem", true).is_err());
        let dir = tempfile::TempDir::new().expect("tempdir");
        let empty = dir.path().join("empty.pem");
        fs::write(&empty, b"").expect("write");
        let tls = outbound_tls_from("", &empty.to_string_lossy(), true).expect("empty file");
        assert!(!tls.builtin_roots, "a verify file replaces the default roots");
    }

    #[test]
    fn missing_verify_dir_fails_like_rippled() {
        assert!(outbound_tls_from("/nonexistent/dir", "", true).is_err());
    }

    #[test]
    fn clients_build_with_the_settings() {
        // Uses the process defaults; tests must not install global settings.
        assert!(apply_outbound_tls(reqwest::Client::builder()).build().is_ok());
        assert!(
            apply_outbound_tls_blocking(reqwest::blocking::Client::builder())
                .build()
                .is_ok()
        );
    }
}
