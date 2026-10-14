//! The DNS seam of the egress guard: the system resolver in the app, scripted answers in tests (DNS-rebinding style
//! answers cannot be produced reliably from a real resolver in a gate).
//!
//! Depends on: tokio (`lookup_host`). Called by: `decide`, the app (system resolver), tests (fakes).

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;

/// A boxed resolver future.
pub type ResolveFuture<'a> = Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>>;

/// Resolves a host name to socket addresses.
pub trait Resolve: Send + Sync {
    /// Every address `host` answers, with `port`.
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
}

/// The operating system's resolver (`getaddrinfo` on a blocking thread, through tokio).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move { Ok(tokio::net::lookup_host((host, port)).await?.collect()) })
    }
}
