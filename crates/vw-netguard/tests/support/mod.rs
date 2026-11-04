//! Local HTTP/HTTPS-CONNECT and SOCKS fixtures; tests own every listener and abort it on drop.
//! These fake endpoints never dial a destination: they record numeric targets and echo tunnel traffic.
mod tls;
use std::net::SocketAddr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
pub type Stream = Box<dyn Io>;
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Http,
    Https,
    Socks4,
    Socks5,
}
#[cfg(target_os = "linux")]
pub fn ca_pem() -> &'static str {
    &tls::fixtures().ca_pem
}
pub fn ca() -> &'static [u8] {
    &tls::fixtures().ca
}
pub struct Proxy {
    pub address: SocketAddr,
    pub url: String,
    pub connections: Arc<AtomicUsize>,
    pub targets: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Proxy {
    pub async fn new(kind: Kind, auth: bool, refuse: bool) -> Self {
        Self::with_identity(kind, auth, refuse, false).await
    }
    pub async fn wrong_name() -> Self {
        Self::with_identity(Kind::Https, false, false, true).await
    }
    async fn with_identity(kind: Kind, auth: bool, refuse: bool, wrong_name: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let scheme = match kind {
            Kind::Http => "http",
            Kind::Https => "https",
            Kind::Socks4 => "socks4a",
            Kind::Socks5 => "socks5h",
        };
        let credentials = if !auth {
            ""
        } else if matches!(kind, Kind::Socks4) {
            "user@"
        } else {
            "us%65r:se%63ret@"
        };
        let connections = Arc::new(AtomicUsize::new(0));
        let targets = Arc::new(Mutex::new(Vec::new()));
        let (count, seen) = (connections.clone(), targets.clone());
        let tls = tls::acceptor(wrong_name);
        let task = tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let seen = seen.clone();
                let tls = tls.clone();
                tokio::spawn(async move {
                    let mut stream: Stream = if matches!(kind, Kind::Https) {
                        let Ok(tls) = tls.accept(tcp).await else {
                            return;
                        };
                        Box::new(tls)
                    } else {
                        Box::new(tcp)
                    };
                    if !handshake(&mut stream, kind, auth, refuse, &seen).await {
                        return;
                    }
                    let mut bytes = [0; 4096];
                    loop {
                        let n = stream.read(&mut bytes).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        if bytes[..n].starts_with(b"GET ") {
                            let body = String::from_utf8_lossy(&bytes[..n]);
                            let reply = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(reply.as_bytes()).await;
                            let _ = stream.shutdown().await;
                            return;
                        }
                        if stream.write_all(&bytes[..n]).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            address,
            url: format!("{scheme}://{credentials}{address}"),
            connections,
            targets,
            task,
        }
    }
    pub fn count(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

pub async fn head(stream: &mut (impl AsyncRead + Unpin + ?Sized)) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(b) => bytes.push(b),
            Err(_) => break,
        }
        assert!(bytes.len() <= 16384);
    }
    String::from_utf8(bytes).unwrap()
}
#[allow(clippy::too_many_lines)] // The fixture deliberately spells out each wire protocol in one place.
async fn handshake(
    stream: &mut Stream,
    kind: Kind,
    auth: bool,
    refuse: bool,
    targets: &Mutex<Vec<String>>,
) -> bool {
    match kind {
        Kind::Http | Kind::Https => {
            let request = head(stream).await;
            if request.is_empty() {
                return false;
            }
            if auth {
                assert!(request.contains("Proxy-Authorization: Basic dXNlcjpzZWNyZXQ="));
            }
            let words: Vec<_> = request.split_whitespace().collect();
            targets.lock().unwrap().push(words[1].to_owned());
            if refuse {
                let _ = stream
                    .write_all(b"HTTP/1.1 403 No\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return false;
            }
            if words[0] == "CONNECT" {
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            } else {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{request}",
                    request.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                return false;
            }
        }
        Kind::Socks5 => {
            let mut hello = [0; 3];
            stream.read_exact(&mut hello).await.unwrap();
            let method = if auth { 2 } else { 0 };
            assert_eq!(hello, [5, 1, method]);
            stream.write_all(&[5, method]).await.unwrap();
            if auth {
                assert_eq!(stream.read_u8().await.unwrap(), 1);
                let len = stream.read_u8().await.unwrap();
                let mut user = vec![0; len as usize];
                stream.read_exact(&mut user).await.unwrap();
                let len = stream.read_u8().await.unwrap();
                let mut password = vec![0; len as usize];
                stream.read_exact(&mut password).await.unwrap();
                assert_eq!(user, b"user");
                assert_eq!(password, b"secret");
                stream.write_all(&[1, u8::from(refuse)]).await.unwrap();
                if refuse {
                    return false;
                }
            }
            let mut request = [0; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..3], &[5, 1, 0]);
            // A hostname address type would reopen DNS at the proxy: fail the fixture immediately.
            let ip = match request[3] {
                1 => {
                    let mut bytes = [0; 4];
                    stream.read_exact(&mut bytes).await.unwrap();
                    std::net::IpAddr::from(bytes)
                }
                4 => {
                    let mut bytes = [0; 16];
                    stream.read_exact(&mut bytes).await.unwrap();
                    std::net::IpAddr::from(bytes)
                }
                _ => panic!("SOCKS must receive a checked numeric IP"),
            };
            let port = stream.read_u16().await.unwrap();
            targets
                .lock()
                .unwrap()
                .push(SocketAddr::new(ip, port).to_string());
            stream
                .write_all(&[5, u8::from(refuse), 0, 1, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            if refuse {
                return false;
            }
        }
        Kind::Socks4 => {
            let mut request = [0; 8];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..2], &[4, 1]);
            let mut user = Vec::new();
            loop {
                let b = stream.read_u8().await.unwrap();
                if b == 0 {
                    break;
                }
                user.push(b);
            }
            assert_eq!(user, if auth { b"user".as_slice() } else { b"" });
            let ip = std::net::Ipv4Addr::new(request[4], request[5], request[6], request[7]);
            assert_ne!(ip.octets()[..3], [0, 0, 0], "no SOCKS4a hostname marker");
            targets.lock().unwrap().push(
                SocketAddr::new(ip.into(), u16::from_be_bytes([request[2], request[3]]))
                    .to_string(),
            );
            stream
                .write_all(&[0, if refuse { 91 } else { 90 }, 0, 0, 0, 0, 0, 0])
                .await
                .unwrap();
            if refuse {
                return false;
            }
        }
    }
    true
}
