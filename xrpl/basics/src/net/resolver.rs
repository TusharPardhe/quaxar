//! Compatibility boundary for `xrpl/basics/Resolver.h`.

use std::net::SocketAddr;
use std::sync::Arc;

pub type Endpoint = SocketAddr;
pub type ResolveHandler = Arc<dyn Fn(String, Vec<Endpoint>) + Send + Sync + 'static>;

pub trait Resolver: Send + Sync {
    fn stop_async(&self);
    fn stop(&self);
    fn start(&self);
    fn resolve(&self, names: &[String], handler: ResolveHandler);
}
