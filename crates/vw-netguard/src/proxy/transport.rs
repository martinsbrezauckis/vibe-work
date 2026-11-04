//! TCP/TLS connection and HTTP CONNECT framing. Used by Upstream after address checks; never re-resolves targets.
use super::{Error, Kind, Route};
use crate::stream::Stream;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(all(test, target_os = "linux"))]
#[path = "tls_tests.rs"]
mod tests;

pub(super) async fn open(
    route: &Route,
    target: SocketAddr,
    roots: &[Vec<u8>],
    tunnel: bool,
) -> Result<Stream, Error> {
    let tcp = tokio::net::TcpStream::connect((route.host.as_str(), route.port))
        .await
        .map_err(|_| Error::Unreachable)?;
    let mut stream: Stream = if route.kind == Kind::Https {
        tls(&route.host, tcp, roots).await?
    } else {
        Box::new(tcp)
    };
    match route.kind {
        Kind::Http | Kind::Https if tunnel => connect(&mut stream, route, target).await?,
        Kind::Socks4 => super::socks::v4(&mut stream, route, target).await?,
        Kind::Socks5 => super::socks::v5(&mut stream, route, target).await?,
        Kind::Http | Kind::Https => {}
    }
    Ok(stream)
}

#[cfg(target_os = "linux")]
async fn tls(host: &str, tcp: tokio::net::TcpStream, roots: &[Vec<u8>]) -> Result<Stream, Error> {
    use rustls::pki_types::{CertificateDer, ServerName};
    use std::sync::Arc;

    // Choose ring explicitly, including when used without the app's TLS initialization.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new_with_extra_roots(
        roots.iter().cloned().map(CertificateDer::from),
        provider.clone(),
    )
    .map_err(|_| Error::Certificate)?;
    // The verifier combines company roots with the OS store and checks both trust and hostname.
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Certificate)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).map_err(|_| Error::Certificate)?;
    let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|_| Error::Certificate)?;
    Ok(Box::new(stream))
}

#[cfg(not(target_os = "linux"))]
async fn tls(host: &str, tcp: tokio::net::TcpStream, roots: &[Vec<u8>]) -> Result<Stream, Error> {
    let mut builder = native_tls::TlsConnector::builder();
    for der in roots {
        builder.add_root_certificate(
            native_tls::Certificate::from_der(der).map_err(|_| Error::Certificate)?,
        );
    }
    // The OS trust store remains enabled. Never accept invalid certificates or skip hostname checks.
    let connector = builder.build().map_err(|_| Error::Certificate)?;
    let stream = tokio_native_tls::TlsConnector::from(connector)
        .connect(host, tcp)
        .await
        .map_err(|_| Error::Certificate)?;
    Ok(Box::new(stream))
}

async fn connect(stream: &mut Stream, route: &Route, target: SocketAddr) -> Result<(), Error> {
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = &route.auth {
        request.push_str("Proxy-Authorization: ");
        request.push_str(auth);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| Error::Unreachable)?;
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= 8192 {
            return Err(Error::Protocol);
        }
        head.push(stream.read_u8().await.map_err(|_| Error::Protocol)?);
    }
    let text = std::str::from_utf8(&head).map_err(|_| Error::Protocol)?;
    let mut fields = text
        .lines()
        .next()
        .ok_or(Error::Protocol)?
        .split_whitespace();
    if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
        return Err(Error::Protocol);
    }
    match fields.next().and_then(|code| code.parse::<u16>().ok()) {
        Some(200..=299) => Ok(()),
        Some(407) => Err(Error::Authentication),
        Some(400..=599) => Err(Error::Refused),
        _ => Err(Error::Protocol),
    }
}
