//! TCP or TLS byte streams used by serve and the upstream transports. No policy lives here.
use tokio::io::{AsyncRead, AsyncWrite};
pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub(crate) type Stream = Box<dyn Io>;
