//! Strict proxy endpoints and PAC directive lists. Depends on hyper-util; used after policy acceptance.
//! No Debug or remote error text can reveal credentials; unknown list entries fail closed.
use super::Error;
use hyper_util::client::proxy::matcher::Matcher;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Http,
    Https,
    Socks4,
    Socks5,
}
#[derive(Clone)]
pub(crate) struct Route {
    pub(crate) roots: Option<Vec<Vec<u8>>>,
    pub(crate) kind: Kind,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) auth: Option<String>,
    pub(crate) credentials: Option<(String, String)>,
}
impl Route {
    pub(crate) fn parse(raw: &str) -> Result<Self, Error> {
        if raw.len() > 8192 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(Error::Configuration);
        }
        let raw = if raw.starts_with("socks://") {
            raw.replacen("socks://", "socks5://", 1)
        } else if raw.contains("://") {
            raw.to_owned()
        } else {
            format!("http://{raw}")
        };
        let url = url::Url::parse(&raw).map_err(|_| Error::Configuration)?;
        if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
            return Err(Error::Configuration);
        }
        let (kind, default) = match url.scheme() {
            "http" => (Kind::Http, 80),
            "https" => (Kind::Https, 443),
            "socks4" | "socks4a" => (Kind::Socks4, 1080),
            "socks5" | "socks5h" => (Kind::Socks5, 1080),
            _ => return Err(Error::Configuration),
        };
        let matcher = Matcher::builder().all(url.as_str()).build();
        let intercept = matcher
            .intercept(&http::Uri::from_static("https://allowed.invalid/"))
            .ok_or(Error::Configuration)?;
        let uri = intercept.uri();
        Ok(Self {
            roots: None,
            kind,
            host: uri
                .host()
                .ok_or(Error::Configuration)?
                .trim_matches(['[', ']'])
                .to_owned(),
            port: uri.port_u16().unwrap_or(default),
            auth: intercept
                .basic_auth()
                .and_then(|a| a.to_str().ok())
                .map(str::to_owned),
            credentials: intercept
                .raw_auth()
                .map(|(u, p)| (u.to_owned(), p.to_owned())),
        })
    }
}

/// Translate an OS PAC result to ordered URLs. SOCKS is SOCKS4; SOCKS5 supports both address families.
///
/// # Errors
/// An empty, excessive or unknown directive is a configuration error, never an implicit direct connection.
pub fn pac_routes(result: &str) -> Result<Vec<String>, Error> {
    if result.len() > 64 * 1024 {
        return Err(Error::Configuration);
    }
    let mut routes = Vec::new();
    for part in result.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let words: Vec<_> = part.split_whitespace().collect();
        let route = match words.as_slice() {
            ["DIRECT"] => "direct://".to_owned(),
            [kind, address] => {
                let scheme = match *kind {
                    "PROXY" | "HTTP" => "http",
                    "HTTPS" => "https",
                    "SOCKS" | "SOCKS4" => "socks4a",
                    "SOCKS5" => "socks5h",
                    _ => return Err(Error::Configuration),
                };
                let raw = format!("{scheme}://{address}");
                Route::parse(&raw)?;
                raw
            }
            _ => return Err(Error::Configuration),
        };
        routes.push(route);
    }
    if routes.is_empty() || routes.len() > 32 {
        return Err(Error::Configuration);
    }
    Ok(routes)
}
