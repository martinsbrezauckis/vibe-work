//! Bounded protocol inspection for macOS browser CONNECT requests before OS PAC lookup.
//! The first tunneled byte is inspected without consuming it; the caller owns the checked upstream route.

use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::connect_protocol::{Protocol, classify};

pub(crate) const CONNECT_ACK: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Inspection {
    pub protocol: Protocol,
    pub acknowledged: bool,
}

pub(crate) async fn inspect(
    stream: &mut TcpStream,
    rest: &[u8],
    limit: Duration,
) -> Result<Inspection, ()> {
    if let Some(first) = rest.first() {
        return Ok(Inspection {
            protocol: classify(*first)?,
            acknowledged: false,
        });
    }
    stream.write_all(CONNECT_ACK).await.map_err(|_| ())?;
    let mut first = [0];
    let read = tokio::time::timeout(limit, stream.peek(&mut first))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    if read == 0 {
        return Err(());
    }
    Ok(Inspection {
        protocol: classify(first[0])?,
        acknowledged: true,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{CONNECT_ACK, Inspection, inspect};
    use crate::connect_protocol::Protocol;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    async fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn buffered_http_and_tls_bytes_classify_without_early_ack() {
        for (rest, protocol) in [
            (&b"GET / HTTP/1.1\r\n"[..], Protocol::Http),
            (&b"\x16\x03\x03"[..], Protocol::Https),
        ] {
            let (mut client, mut server) = pair().await;
            assert_eq!(
                inspect(&mut server, rest, Duration::from_millis(50)).await,
                Ok(Inspection {
                    protocol,
                    acknowledged: false
                })
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(30), client.read_u8())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn post_ack_http_and_tls_bytes_are_peeked_once_without_consumption() {
        for (first, protocol) in [(b'G', Protocol::Http), (0x16, Protocol::Https)] {
            let (mut client, mut server) = pair().await;
            let inspection = tokio::spawn(async move {
                let found = inspect(&mut server, &[], Duration::from_secs(1)).await;
                (found, server)
            });
            let mut ack = vec![0; CONNECT_ACK.len()];
            tokio::time::timeout(Duration::from_secs(1), client.read_exact(&mut ack))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ack, CONNECT_ACK);
            client.write_all(&[first]).await.unwrap();
            let (found, server) = inspection.await.unwrap();
            assert_eq!(
                found,
                Ok(Inspection {
                    protocol,
                    acknowledged: true
                })
            );
            let mut peeked = [0];
            server.peek(&mut peeked).await.unwrap();
            assert_eq!(
                peeked[0], first,
                "inspection must leave the protocol byte for the upstream"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(30), client.read_u8())
                    .await
                    .is_err(),
                "only one CONNECT acknowledgment"
            );
        }
    }

    #[tokio::test]
    async fn unknown_byte_timeout_and_eof_fail_closed() {
        let (_client, mut server) = pair().await;
        assert_eq!(
            inspect(&mut server, b"?", Duration::from_millis(20)).await,
            Err(())
        );
        let (mut client, mut server) = pair().await;
        let inspection =
            tokio::spawn(async move { inspect(&mut server, &[], Duration::from_millis(30)).await });
        let mut ack = vec![0; CONNECT_ACK.len()];
        client.read_exact(&mut ack).await.unwrap();
        assert_eq!(
            inspection.await.unwrap(),
            Err(()),
            "no payload must time out"
        );
        let (mut client, mut server) = pair().await;
        let inspection =
            tokio::spawn(async move { inspect(&mut server, &[], Duration::from_secs(1)).await });
        client.read_exact(&mut ack).await.unwrap();
        client.shutdown().await.unwrap();
        assert_eq!(
            inspection.await.unwrap(),
            Err(()),
            "EOF must not become DIRECT"
        );
    }
}
