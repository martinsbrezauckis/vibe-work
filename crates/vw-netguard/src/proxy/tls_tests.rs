//! Checks the actual Linux connector with one valid leaf at a matching and a mismatching name.
//! Uses the local proxy test identity, so hostname refusal cannot be masked by an invalid leaf purpose.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../../tests/support/tls.rs"]
mod fixture;

#[tokio::test]
async fn trusted_leaf_requires_the_matching_hostname() {
    for (host, accepted) in [("localhost", true), ("wrong-proxy.invalid", false)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let acceptor = fixture::acceptor(false);
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let _ = acceptor.accept(tcp).await;
        });
        let tcp = tokio::net::TcpStream::connect(address).await.unwrap();
        let roots = [fixture::fixtures().ca.clone()];
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            super::tls(host, tcp, &roots),
        )
        .await
        .unwrap();
        assert_eq!(result.is_ok(), accepted, "TLS name: {host}");
        if !accepted {
            assert!(matches!(result, Err(super::Error::Certificate)));
        }
        server.abort();
    }
}
