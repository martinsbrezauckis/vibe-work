//! Native proxy endpoint and in-memory sign-in, consumed by the three webview backends.
//! Debug omits credentials; browser process arguments contain only the endpoint.

#[derive(Clone)]
pub struct ProxyEndpoint {
  /// Proxy server host (e.g. localhost, example.com, etc.)
  pub host: String,
  /// Proxy server port (e.g. 1080, 3128, etc.)
  pub port: String,
  /// Proxy-only credentials, never an origin-server sign-in.
  pub credentials: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub enum ProxyConfig {
  /// Connect to proxy server via HTTP CONNECT
  Http(ProxyEndpoint),
  /// Connect to proxy server via SOCKSv5
  Socks5(ProxyEndpoint),
}

impl std::fmt::Debug for ProxyEndpoint {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ProxyEndpoint")
      .field("host", &self.host)
      .field("port", &self.port)
      .finish_non_exhaustive()
  }
}
