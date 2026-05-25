//! WorkSSL: async HTTPS GET over TLS.
//!

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;

use super::work::{HttpResponse, Work, WorkCallback, WorkError};

/// Async HTTPS client over TLS.
pub struct WorkSSL {
    host: String,
    path: String,
    port: String,
    callback: Arc<Mutex<Option<WorkCallback>>>,
    cancel_token: tokio_util::sync::CancellationToken,
    handle: Handle,
}

impl WorkSSL {
    pub fn new(
        host: String,
        path: String,
        port: String,
        handle: Handle,
        cb: WorkCallback,
    ) -> Arc<Self> {
        Arc::new(Self {
            host,
            path,
            port,
            callback: Arc::new(Mutex::new(Some(cb))),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            handle,
        })
    }

    fn fire_callback(&self, result: Result<(SocketAddr, HttpResponse), WorkError>) {
        if let Some(cb) = self.callback.lock().unwrap().take() {
            cb(result);
        }
    }
}

impl Work for WorkSSL {
    fn run(&self) {
        let host = self.host.clone();
        let path = self.path.clone();
        let port = self.port.clone();
        let callback = self.callback.clone();
        let token = self.cancel_token.clone();

        self.handle.spawn(async move {
            let url = format!("https://{}:{}{}", host, port, if path.is_empty() { "/" } else { &path });

            let result = tokio::select! {
                _ = token.cancelled() => Err(WorkError::Cancelled),
                res = async {
                    let client = reqwest::Client::builder()
                        .use_rustls_tls()
                        .min_tls_version(reqwest::tls::Version::TLS_1_2)
                        .build()
                        .map_err(|e| WorkError::Tls(e.to_string()))?;

                    let resp = client
                        .get(&url)
                        .header("Host", format!("{}:{}", host, port))
                        .header("User-Agent", "xrpld-rust/0.1")
                        .send()
                        .await
                        .map_err(|e| WorkError::Http(e.to_string()))?;

                    let addr = resp.remote_addr().unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)));
                    let status = resp.status().as_u16();
                    let headers: Vec<(String, String)> = resp.headers().iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect();
                    let body = resp.text().await.map_err(|e| WorkError::Http(e.to_string()))?;

                    Ok((addr, HttpResponse { status, headers, body }))
                } => res,
            };

            if let Some(cb) = callback.lock().unwrap().take() {
                cb(result);
            }
        });
    }

    fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

impl Drop for WorkSSL {
    fn drop(&mut self) {
        self.fire_callback(Err(WorkError::Dropped));
    }
}
