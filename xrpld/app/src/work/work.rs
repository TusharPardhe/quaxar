//! Work trait and shared types for async HTTP client.
//!

use std::net::SocketAddr;

/// HTTP response with status, headers, and body.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Errors that can occur during work execution.
#[derive(Debug, Clone)]
pub enum WorkError {
    Dns(String),
    Connect(String),
    Tls(String),
    Http(String),
    Io(String),
    Cancelled,
    Dropped,
}

impl std::fmt::Display for WorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dns(e) => write!(f, "DNS resolution failed: {}", e),
            Self::Connect(e) => write!(f, "connection failed: {}", e),
            Self::Tls(e) => write!(f, "TLS handshake failed: {}", e),
            Self::Http(e) => write!(f, "HTTP error: {}", e),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Dropped => write!(f, "work dropped before completion"),
        }
    }
}

impl std::error::Error for WorkError {}

/// Callback type for HTTP work completion.
pub type WorkCallback =
    Box<dyn FnOnce(Result<(SocketAddr, HttpResponse), WorkError>) + Send + 'static>;

/// Callback type for file work completion.
pub type FileWorkCallback = Box<dyn FnOnce(Result<String, WorkError>) + Send + 'static>;

/// Async work unit trait.
pub trait Work: Send + Sync {
    fn run(&self);
    fn cancel(&self);
}
