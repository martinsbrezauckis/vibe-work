//! Whether sound can leave the app, for the `audioOutput` speech request: read aloud and voice chat show "no audio
//! output" instead of playing into nothing.
//!
//! Why the host decides: in the Linux `AppImage` the bundled `GStreamer` has exactly one audio sink, `PulseAudio`'s
//! (`gst_plugins.py`). Without a sound server `GStreamer`'s `autoaudiosink` falls back to a silent fake sink, so the
//! page plays normally and cannot tell. The check looks for the server the way libpulse does: `PULSE_SERVER`, a
//! `default-server` in `client.conf`, the user's runtime socket, the system socket, and tries to connect.
//! Everywhere else (the deb with the system's `GStreamer`, which may play through ALSA or `PipeWire` directly; macOS;
//! Windows) the answer is always "available": there is no reliable negative there.
//!
//! Depends on: std only. Called by: `speech` (`SpeechRequest::AudioOutput`).
//! Invariants: never blocks longer than one short TCP timeout per configured network server; reads no file other
//! than `PulseAudio`'s `client.conf` files; a doubt answers "available" (never a false "no audio output").

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a configured network sound server may take to accept.
const TCP_TIMEOUT: Duration = Duration::from_millis(400);
/// `PulseAudio`'s default TCP port.
const PULSE_PORT: u16 = 4713;

/// One place a `PulseAudio` client would connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Server {
    /// A local socket.
    Unix(PathBuf),
    /// A network server (`tcp:host[:port]`, or a plain host name).
    Tcp(String, u16),
}

/// True unless this is the Linux `AppImage` and no sound server answers.
pub(crate) fn available() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    decide(&env, &read_client_conf, &reachable)
}

/// The decision, with the environment, `client.conf` reading and connecting passed in (tests use doubles).
pub(crate) fn decide(
    env: &dyn Fn(&str) -> Option<String>,
    client_conf: &dyn Fn(&Path) -> Option<String>,
    connect: &dyn Fn(&Server) -> bool,
) -> bool {
    // Only the AppImage's GStreamer is limited to the PulseAudio sink (the AppImage runtime sets APPIMAGE).
    if env("APPIMAGE").is_none() {
        return true;
    }
    servers(env, client_conf).iter().any(connect)
}

/// The servers libpulse would try, in its order.
pub(crate) fn servers(
    env: &dyn Fn(&str) -> Option<String>,
    client_conf: &dyn Fn(&Path) -> Option<String>,
) -> Vec<Server> {
    if let Some(list) = env("PULSE_SERVER") {
        return parse_list(&list);
    }
    if let Some(list) = conf_default_server(env, client_conf) {
        return parse_list(&list);
    }
    let mut out = Vec::new();
    if let Some(dir) = env("PULSE_RUNTIME_PATH") {
        out.push(Server::Unix(Path::new(&dir).join("native")));
    } else if let Some(dir) = env("XDG_RUNTIME_DIR") {
        out.push(Server::Unix(Path::new(&dir).join("pulse/native")));
    }
    out.push(Server::Unix(PathBuf::from("/run/pulse/native")));
    out.push(Server::Unix(PathBuf::from("/var/run/pulse/native")));
    out
}

/// `default-server` from the first `client.conf` that sets it (`PULSE_CLIENTCONFIG`, the user's, then the system's).
fn conf_default_server(
    env: &dyn Fn(&str) -> Option<String>,
    client_conf: &dyn Fn(&Path) -> Option<String>,
) -> Option<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(file) = env("PULSE_CLIENTCONFIG") {
        files.push(PathBuf::from(file));
    } else {
        if let Some(dir) = env("XDG_CONFIG_HOME") {
            files.push(Path::new(&dir).join("pulse/client.conf"));
        } else if let Some(home) = env("HOME") {
            files.push(Path::new(&home).join(".config/pulse/client.conf"));
        }
        files.push(PathBuf::from("/etc/pulse/client.conf"));
    }
    files.iter().find_map(|file| {
        client_conf(file)?.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            (key.trim() == "default-server" && !value.is_empty()).then(|| value.to_owned())
        })
    })
}

/// A `PulseAudio` server list: space-separated entries, each `unix:/path`, `/path`, `tcp:host[:port]`,
/// `tcp6:...`, or a plain host. A `{machine-id}` prefix is dropped (it only limits which machine uses the entry).
fn parse_list(list: &str) -> Vec<Server> {
    list.split_whitespace()
        .filter_map(|entry| {
            let entry = match entry.strip_prefix('{') {
                Some(rest) => rest.split_once('}').map_or("", |(_, e)| e),
                None => entry,
            };
            if let Some(path) = entry.strip_prefix("unix:") {
                return Some(Server::Unix(PathBuf::from(path)));
            }
            if entry.starts_with('/') {
                return Some(Server::Unix(PathBuf::from(entry)));
            }
            let host = entry
                .strip_prefix("tcp:")
                .or_else(|| entry.strip_prefix("tcp4:"))
                .or_else(|| entry.strip_prefix("tcp6:"))
                .unwrap_or(entry);
            if host.is_empty() {
                return None;
            }
            // `host:port`, but a bare IPv6 address has colons of its own (`[::1]:4713` keeps its brackets).
            match host.rsplit_once(':') {
                Some((name, port)) if !name.contains(':') || name.ends_with(']') => {
                    port.parse().ok().map(|p| Server::Tcp(name.to_owned(), p))
                }
                _ => Some(Server::Tcp(host.to_owned(), PULSE_PORT)),
            }
        })
        .collect()
}

fn read_client_conf(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn reachable(server: &Server) -> bool {
    match server {
        #[cfg(unix)]
        Server::Unix(path) => std::os::unix::net::UnixStream::connect(path).is_ok(),
        #[cfg(not(unix))]
        Server::Unix(_) => false,
        Server::Tcp(host, port) => {
            let name = host.trim_start_matches('[').trim_end_matches(']');
            (name, *port).to_socket_addrs().is_ok_and(|mut addrs| {
                addrs.any(|addr| TcpStream::connect_timeout(&addr, TCP_TIMEOUT).is_ok())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }
    fn no_conf(_: &Path) -> Option<String> {
        None
    }

    #[test]
    fn outside_the_appimage_sound_is_always_available() {
        let env = env_of(&[("XDG_RUNTIME_DIR", "/run/user/1000")]);
        assert!(decide(&env, &no_conf, &|_| false));
    }

    #[test]
    fn in_the_appimage_without_a_server_there_is_no_output() {
        let env = env_of(&[
            ("APPIMAGE", "/a/Vibe.AppImage"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        assert!(!decide(&env, &no_conf, &|_| false));
    }

    #[test]
    fn in_the_appimage_the_user_socket_is_tried_first() {
        let env = env_of(&[
            ("APPIMAGE", "/a/Vibe.AppImage"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        let user = Server::Unix(PathBuf::from("/run/user/1000/pulse/native"));
        assert_eq!(servers(&env, &no_conf)[0], user);
        assert!(decide(&env, &no_conf, &|s| *s == user));
    }

    #[test]
    fn pulse_server_replaces_the_defaults() {
        let env = env_of(&[
            ("APPIMAGE", "x"),
            (
                "PULSE_SERVER",
                "unix:/tmp/missing {abc}tcp:sound.lan:4800 /tmp/p tcp6:[::1]:4713",
            ),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]);
        assert_eq!(
            servers(&env, &no_conf),
            vec![
                Server::Unix(PathBuf::from("/tmp/missing")),
                Server::Tcp("sound.lan".into(), 4800),
                Server::Unix(PathBuf::from("/tmp/p")),
                Server::Tcp("[::1]".into(), 4713),
            ]
        );
        // The runtime socket is not tried once PULSE_SERVER names others (libpulse does the same).
        let runtime = Server::Unix(PathBuf::from("/run/user/1000/pulse/native"));
        assert!(!decide(&env, &no_conf, &|s| *s == runtime));
    }

    #[test]
    fn a_plain_host_uses_the_default_port() {
        let env = env_of(&[("PULSE_SERVER", "soundbox")]);
        assert_eq!(
            servers(&env, &no_conf),
            vec![Server::Tcp("soundbox".into(), PULSE_PORT)]
        );
    }

    #[test]
    fn client_conf_default_server_is_used() {
        let env = env_of(&[
            ("APPIMAGE", "x"),
            ("PULSE_CLIENTCONFIG", "/cfg/client.conf"),
        ]);
        let conf = |p: &Path| {
            (p == Path::new("/cfg/client.conf"))
                .then(|| "; comment\nautospawn = no\ndefault-server = unix:/srv/pa\n".into())
        };
        assert_eq!(
            servers(&env, &conf),
            vec![Server::Unix(PathBuf::from("/srv/pa"))]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_real_listening_socket_counts_and_a_missing_one_does_not() {
        let dir = std::env::temp_dir().join(format!("vw-audio-output-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("native");
        let _ = std::fs::remove_file(&socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert!(reachable(&Server::Unix(socket.clone())));
        drop(listener);
        std::fs::remove_file(&socket).unwrap();
        assert!(!reachable(&Server::Unix(socket)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
