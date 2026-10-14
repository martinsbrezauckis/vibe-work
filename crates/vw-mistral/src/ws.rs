//! The WebSocket connection for Voxtral realtime: TCP, then TLS opened by us with the TLS
//! stack the rest of the crate uses (Linux: rustls + ring + the OS trust store through
//! `rustls-platform-verifier`, like reqwest's client; Windows/macOS: the OS TLS stack through
//! `native-tls`), then the WebSocket handshake by `tokio-tungstenite` with the key header.
//! `ws://` (plain) exists for the loopback fake in tests; the app only builds `wss://` URLs.
//!
//! Depends on: tokio, tokio-tungstenite (handshake only), the platform TLS crates. Called by: `realtime`.
//! Invariants: the `Authorization` value is passed in already marked sensitive and is never logged or put
//! in an error; a failed handshake reports only its HTTP status; no redirect is followed (tungstenite does
//! not follow them).

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderValue};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use crate::tts::SpeechError;

/// Any byte stream the WebSocket can run over (plain TCP or TLS).
pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// An open WebSocket.
pub(crate) type Socket = WebSocketStream<Box<dyn Io>>;

fn net(reason: &str) -> SpeechError {
    SpeechError::Network(reason.to_owned())
}

#[cfg(target_os = "linux")]
async fn tls(host: &str, tcp: TcpStream) -> Result<Box<dyn Io>, SpeechError> {
    use rustls_platform_verifier::ConfigVerifierExt as _;
    // Same provider choice as `client.rs` (installing fails only when one is installed already).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = rustls::ClientConfig::with_platform_verifier()
        .map_err(|_| net("the system certificates could not be loaded"))?;
    // A WebSocket upgrade is HTTP/1.1; never let ALPN pick h2.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| net("the host name is not valid"))?;
    let stream = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|_| net("the secure connection failed"))?;
    Ok(Box::new(stream))
}

#[cfg(not(target_os = "linux"))]
async fn tls(host: &str, tcp: TcpStream) -> Result<Box<dyn Io>, SpeechError> {
    let connector =
        native_tls::TlsConnector::new().map_err(|_| net("the system TLS could not be set up"))?;
    let stream = tokio_native_tls::TlsConnector::from(connector)
        .connect(host, tcp)
        .await
        .map_err(|_| net("the secure connection failed"))?;
    Ok(Box::new(stream))
}

/// Opens `url` (`wss://` or `ws://`) with `auth` as the `Authorization` header, within `timeout` for TCP,
/// TLS and the handshake together.
///
/// # Errors
/// [`SpeechError::Network`] for connection failures; the handshake's HTTP status mapped like the REST
/// calls (`401` -> [`SpeechError::Unauthorized`],...).
pub(crate) async fn connect(
    url: &str,
    auth: HeaderValue,
    timeout: Duration,
) -> Result<Socket, SpeechError> {
    tokio::time::timeout(timeout, connect_inner(url, auth))
        .await
        .map_err(|_| net("the connection timed out"))?
}

async fn connect_inner(url: &str, auth: HeaderValue) -> Result<Socket, SpeechError> {
    let mut request = url
        .into_client_request()
        .map_err(|_| net("the address is not valid"))?;
    request.headers_mut().insert(AUTHORIZATION, auth);
    let uri = request.uri().clone();
    let secure = match uri.scheme_str() {
        Some("wss") => true,
        Some("ws") => false,
        _ => return Err(net("the address is not a WebSocket address")),
    };
    let host = uri
        .host()
        .ok_or_else(|| net("the address has no host"))?
        .to_owned();
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|_| net("the connection failed"))?;
    let _ = tcp.set_nodelay(true);
    let stream: Box<dyn Io> = if secure {
        tls(&host, tcp).await?
    } else {
        Box::new(tcp)
    };
    match tokio_tungstenite::client_async(request, stream).await {
        Ok((socket, _response)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            Err(crate::tts::status_error(response.status().as_u16()))
        }
        Err(_) => Err(net("the live transcription connection could not be opened")),
    }
}
