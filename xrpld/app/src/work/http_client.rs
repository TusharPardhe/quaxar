//! Async HTTP(S) client with site failover, timeout, and response size limits.
//!
//!
//! Key behaviors preserved:
//! - Tries sites in order; on failure, calls completion callback which returns
//!   `true` to try the next site or `false` to stop.
//! - Enforces `response_max` byte limit (uses Content-Length header or default).
//! - Enforces a per-request timeout.
//! - Supports both HTTP and HTTPS (via reqwest with rustls).

use std::collections::VecDeque;
use std::time::Duration;

/// Max header size: 32 KB (matches reference `kMaxClientHeaderBytes`).
pub const MAX_CLIENT_HEADER_BYTES: usize = 32 * 1024;

/// Result of an HTTP request.
#[derive(Debug, Clone)]
pub struct HttpResult {
    pub status: u16,
    pub body: String,
}

/// Error from an HTTP request attempt.
#[derive(Debug, Clone)]
pub enum HttpClientError {
    Timeout,
    TooLarge,
    Network(String),
    NoStatusCode,
}

impl std::fmt::Display for HttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "request timed out"),
            Self::TooLarge => write!(f, "response too large"),
            Self::Network(e) => write!(f, "network error: {}", e),
            Self::NoStatusCode => write!(f, "no status code in response"),
        }
    }
}

/// Completion callback type.
/// Returns `true` to try the next site on error, `false` to stop.
///
pub type CompletionFn =
    Box<dyn FnMut(Result<HttpResult, HttpClientError>) -> bool + Send + 'static>;

/// Async HTTP client with site failover.
///
pub async fn http_client_get(
    ssl: bool,
    sites: Vec<String>,
    port: u16,
    path: &str,
    response_max: usize,
    timeout: Duration,
    mut complete: CompletionFn,
) {
    let mut sites: VecDeque<String> = sites.into();
    let scheme = if ssl { "https" } else { "http" };

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    while let Some(site) = sites.pop_front() {
        let url = format!(
            "{}://{}:{}{}",
            scheme,
            site,
            port,
            if path.is_empty() { "/" } else { path }
        );

        let result = match client
            .get(&url)
            .header("Host", format!("{}:{}", site, port))
            .header("User-Agent", "xrpld-rust/0.1")
            .header("Accept", "*/*")
            .header("Connection", "close")
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();

                // Check Content-Length against response_max
                if let Some(cl) = resp.content_length() {
                    if cl as usize > response_max {
                        Err(HttpClientError::TooLarge)
                    } else {
                        match resp.text().await {
                            Ok(body) => {
                                if body.len() > response_max {
                                    Err(HttpClientError::TooLarge)
                                } else {
                                    Ok(HttpResult { status, body })
                                }
                            }
                            Err(e) => Err(HttpClientError::Network(e.to_string())),
                        }
                    }
                } else {
                    match resp.text().await {
                        Ok(body) => {
                            if body.len() > response_max {
                                Err(HttpClientError::TooLarge)
                            } else {
                                Ok(HttpResult { status, body })
                            }
                        }
                        Err(e) => Err(HttpClientError::Network(e.to_string())),
                    }
                }
            }
            Err(e) => {
                if e.is_timeout() {
                    Err(HttpClientError::Timeout)
                } else {
                    Err(HttpClientError::Network(e.to_string()))
                }
            }
        };

        let try_next = complete(result);
        if !try_next || sites.is_empty() {
            break;
        }
    }
}

/// Blocking variant for use outside async contexts.
///
/// synchronously in many call sites.
pub fn http_client_get_blocking(
    ssl: bool,
    sites: Vec<String>,
    port: u16,
    path: &str,
    response_max: usize,
    timeout: Duration,
    complete: CompletionFn,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime for HTTPClient");

    rt.block_on(http_client_get(
        ssl,
        sites,
        port,
        path,
        response_max,
        timeout,
        complete,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sites_does_not_panic() {
        http_client_get_blocking(
            false,
            vec![],
            80,
            "/",
            1024,
            Duration::from_secs(1),
            Box::new(|_| false),
        );
    }
}
