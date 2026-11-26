//! Byte relay after a checked CONNECT route has opened.

use super::{Decided, HEAD_LIMIT, answer};
use crate::head;
use tokio::io::{AsyncWriteExt, copy_bidirectional};
use tokio::net::TcpStream;

pub(super) async fn relay_tunnel(
    decided: &Decided<'_>,
    mut stream: TcpStream,
    mut upstream: crate::stream::Stream,
    head: &head::Head,
    rule: Option<crate::ws_origin::Rule<'_>>,
    #[cfg(feature = "proxy-transports")] acknowledged: bool,
) {
    // One target by nature: the checked addresses (or the proxy's tunnel to a checked address), connected above.
    #[cfg(feature = "proxy-transports")]
    if !acknowledged
        && stream
            .write_all(crate::connect_tunnel::CONNECT_ACK)
            .await
            .is_err()
    {
        return;
    }
    #[cfg(not(feature = "proxy-transports"))]
    if stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .is_err()
    {
        return;
    }
    let first = if let Some(rule) = rule {
        match tokio::time::timeout(
            HEAD_LIMIT,
            crate::ws_origin::tunnel_start(&mut stream, head.rest.clone(), rule),
        )
        .await
        {
            Ok(Ok(first)) => first,
            Ok(Err(reason)) => {
                answer(&mut stream, "403 Forbidden", &decided.refuse(&reason)).await;
                return;
            }
            Err(_) => return,
        }
    } else {
        head.rest.clone()
    };
    if upstream.write_all(&first).await.is_err() {
        return;
    }
    let _ = copy_bidirectional(&mut stream, &mut upstream).await;
}
