//! Async HTTP(S) client for fetching validator lists, UNL sites, and remote resources.
//!
//! - `src/xrpld/app/misc/detail/Work.h`, `WorkBase.h`, `WorkPlain.h`, `WorkSSL.h`, `WorkFile.h`
//! - `src/libxrpl/net/the reference source` (site-failover HTTP client)
//! - `src/libxrpl/net/the reference source` (TLS root cert loading)
//! - `include/xrpl/net/HTTPClientSSLContext.h` (SSL context with verification)

pub mod http_client;
pub mod setup_hash_router;
pub mod ssl_certs;
pub mod work;
pub mod work_file;
pub mod work_plain;
pub mod work_ssl;

pub use http_client::*;
pub use setup_hash_router::*;
pub use ssl_certs::*;
pub use work::*;
pub use work_file::*;
pub use work_plain::*;
pub use work_ssl::*;
