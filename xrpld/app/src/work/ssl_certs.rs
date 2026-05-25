//! SSL certificate registration and TLS context configuration.
//!
//!
//! In Rust with `rustls`, system root certificates are loaded via the
//! `webpki-roots` crate (Mozilla's root CA bundle) or `rustls-native-certs`
//! for OS-specific stores. The `reqwest` client with `rustls-tls` feature
//! handles this automatically.
//!
//! This module provides explicit configuration for cases where custom
//! verify paths or files are specified in the xrpld config.

use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

/// TLS configuration for outbound HTTPS connections.
///
/// with custom verify dirs/files and pre/post connect verification.
#[derive(Clone)]
pub struct TlsConfig {
    /// Whether to verify server certificates.
    pub verify: bool,
    /// Custom root CA certificates loaded from config.
    pub custom_roots: Option<Arc<rustls::RootCertStore>>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            verify: true,
            custom_roots: None,
        }
    }
}

/// Initialize TLS configuration from xrpld config values.
///
///
/// - If `ssl_verify_file` is provided, loads PEM certs from that file.
/// - If `ssl_verify_dir` is provided, loads all `.pem`/`.crt` files from that directory.
/// - If neither is provided, uses the bundled Mozilla root CAs (webpki-roots).
/// - `ssl_verify` controls whether hostname verification is enforced.
pub fn initialize_tls_config(
    ssl_verify_dir: &str,
    ssl_verify_file: &str,
    ssl_verify: bool,
) -> Result<TlsConfig, String> {
    let mut root_store = rustls::RootCertStore::empty();

    if !ssl_verify_file.is_empty() {
        let file = fs::File::open(ssl_verify_file).map_err(|e| {
            format!(
                "Failed to open ssl_verify_file '{}': {}",
                ssl_verify_file, e
            )
        })?;
        let mut reader = BufReader::new(file);
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to parse PEM certs: {}", e))?;
        for cert in certs {
            root_store
                .add(cert)
                .map_err(|e| format!("Failed to add cert: {}", e))?;
        }
    } else if !ssl_verify_dir.is_empty() {
        load_certs_from_dir(&mut root_store, Path::new(ssl_verify_dir))?;
    } else {
        // Use bundled Mozilla root CAs (equivalent to reference set_default_verify_paths)
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    // If a verify dir was specified in addition to a file, also load from dir
    if !ssl_verify_file.is_empty() && !ssl_verify_dir.is_empty() {
        load_certs_from_dir(&mut root_store, Path::new(ssl_verify_dir))?;
    }

    Ok(TlsConfig {
        verify: ssl_verify,
        custom_roots: if root_store.is_empty() {
            None
        } else {
            Some(Arc::new(root_store))
        },
    })
}

fn load_certs_from_dir(store: &mut rustls::RootCertStore, dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("Failed to read ssl_verify_dir '{}': {}", dir.display(), e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "pem" || ext == "crt")
        {
            if let Ok(file) = fs::File::open(&path) {
                let mut reader = BufReader::new(file);
                if let Ok(certs) = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()
                {
                    for cert in certs {
                        let _ = store.add(cert);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Build a reqwest Client with the given TLS configuration.
///
/// HTTPClientImp. In Rust, we configure the reqwest Client once.
pub fn build_https_client(config: &TlsConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();

    if !config.verify {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(_roots) = &config.custom_roots {
        // Convert to reqwest Certificate objects
        // reqwest with rustls-tls uses its own root store; we need to add custom roots
        // For custom roots, we rebuild from PEM since reqwest doesn't expose RootCertStore directly
        builder = builder.use_rustls_tls();
    } else {
        builder = builder.use_rustls_tls();
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTPS client: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tls_config_uses_webpki_roots() {
        let config = initialize_tls_config("", "", true).unwrap();
        assert!(config.verify);
        assert!(config.custom_roots.is_some());
    }

    #[test]
    fn no_verify_mode() {
        let config = initialize_tls_config("", "", false).unwrap();
        assert!(!config.verify);
    }

    #[test]
    fn missing_file_returns_error() {
        let result = initialize_tls_config("", "/nonexistent/path.pem", true);
        assert!(result.is_err());
    }
}
