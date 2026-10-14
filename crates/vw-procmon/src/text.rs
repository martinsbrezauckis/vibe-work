//! Pure text scanning of process output: local URLs a dev server prints ("Local: <http://localhost:5173>/")
//! and "address already in use" lines.
//!
//! Depends on: std only. Called by: `attribution` (URL corroboration), `explain` (busy-port lines), tests.
//! Only loopback and wildcard hosts count as local (`localhost`, `127.0.0.1`, `0.0.0.0`, `[::1]`, `[::]`):
//! a URL to any other host is not a port on this machine.

/// A local URL (or `host:port`) found in output text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalUrl {
    /// The port.
    pub port: u16,
    /// The text as it appeared (scheme included when present), clipped to 200 characters.
    pub text: String,
}

const HOSTS: [&str; 5] = ["localhost:", "127.0.0.1:", "0.0.0.0:", "[::1]:", "[::]:"];

/// Every local URL or `host:port` in `text`, in order of appearance, duplicates removed (same port and
/// text).
#[must_use]
pub fn local_urls(text: &str) -> Vec<LocalUrl> {
    let lower = text.to_ascii_lowercase();
    let mut found: Vec<(usize, LocalUrl)> = Vec::new();
    for host in HOSTS {
        let mut from = 0;
        while let Some(offset) = lower.get(from..).and_then(|rest| rest.find(host)) {
            let at = from + offset;
            from = at + host.len();
            // "notlocalhost:80" is not localhost; a host must start at a boundary.
            if at > 0
                && lower
                    .as_bytes()
                    .get(at - 1)
                    .is_some_and(u8::is_ascii_alphanumeric)
            {
                continue;
            }
            let Some(port) = leading_port(lower.get(from..).unwrap_or_default()) else {
                continue;
            };
            let start = scheme_start(&lower, at);
            let end = url_end(&lower, from);
            let shown: String = text
                .get(start..end)
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            found.push((at, LocalUrl { port, text: shown }));
        }
    }
    found.sort_by_key(|(at, _)| *at);
    let mut urls: Vec<LocalUrl> = Vec::new();
    for (_, url) in found {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

/// Parses decimal digits at the start of `rest` as a port (1..=65535); the digits must not continue into
/// letters (`:8080abc` is not a port).
fn leading_port(rest: &str) -> Option<u16> {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 5 {
        return None;
    }
    if rest
        .chars()
        .nth(digits.len())
        .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    digits.parse::<u16>().ok().filter(|p| *p > 0)
}

/// Start of the URL: includes `http://` / `https://` right before the host when present.
fn scheme_start(lower: &str, host_at: usize) -> usize {
    for scheme in ["http://", "https://"] {
        if host_at >= scheme.len() && lower.get(host_at - scheme.len()..host_at) == Some(scheme) {
            return host_at - scheme.len();
        }
    }
    host_at
}

/// End of the URL: the first whitespace, quote or bracket-like delimiter after the port.
fn url_end(lower: &str, port_at: usize) -> usize {
    lower
        .get(port_at..)
        .and_then(|rest| rest.find(|c: char| c.is_whitespace() || "\"'<>()`,".contains(c)))
        .map_or(lower.len(), |n| port_at + n)
}

/// What an "address already in use" line says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusyLine {
    /// The port, when the line names it (Python's `[Errno 98]` line does not).
    pub port: Option<u16>,
}

/// Recognises an "address already in use" line (Node `EADDRINUSE`, Python `Errno 98` / `Errno 48` /
/// `WinError 10048`, Go and Rust `address already in use`, Vite / Next `Port N is in use`) and extracts the
/// port when the line names one. `None` when the line is not such a line.
#[must_use]
pub fn busy_port_line(line: &str) -> Option<BusyLine> {
    let lower = line.to_ascii_lowercase();
    let busy = lower.contains("address already in use")
        || lower.contains("eaddrinuse")
        || lower.contains("errno 98")
        || lower.contains("errno 48")
        || lower.contains("10048")
        || lower.contains("only one usage of each socket address")
        || (lower.contains("port ") && lower.contains(" is in use"))
        || (lower.contains("port ") && lower.contains("already in use"));
    if !busy {
        return None;
    }
    Some(BusyLine {
        port: port_after_word(&lower, "port ").or_else(|| last_colon_port(&lower)),
    })
}

/// "port 5173" -> 5173 (first occurrence followed by digits).
fn port_after_word(lower: &str, word: &str) -> Option<u16> {
    let mut from = 0;
    while let Some(offset) = lower.get(from..).and_then(|rest| rest.find(word)) {
        let at = from + offset + word.len();
        if let Some(port) = leading_port(lower.get(at..).unwrap_or_default()) {
            return Some(port);
        }
        from = at;
    }
    None
}

/// The last `:<digits>` group that is a whole port (`:::3000` -> 3000, `127.0.0.1:8080: bind` -> 8080).
/// Groups below 100 are skipped: they are far more often a clock (`12:34:56`) than a dev-server port.
fn last_colon_port(lower: &str) -> Option<u16> {
    lower
        .match_indices(':')
        .filter_map(|(at, _)| leading_port(lower.get(at + 1..).unwrap_or_default()))
        .rfind(|port| *port >= 100)
}
