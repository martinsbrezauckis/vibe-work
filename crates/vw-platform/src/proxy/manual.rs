//! OS manual proxy lists and credential-safe endpoint construction. Used by Windows and macOS adapters.
//! Unknown settings fail closed; bypass rules only affect transport after destination policy checks.

pub(super) fn endpoint(
    scheme: &str,
    host: &str,
    port: u16,
    user: Option<&str>,
    password: Option<&str>,
) -> Result<String, &'static str> {
    let mut url = url::Url::parse(&format!("{scheme}://placeholder.invalid"))
        .map_err(|_| "Invalid system proxy.")?;
    if let Ok(ip) = host.parse() {
        url.set_ip_host(ip).map_err(|()| "Invalid system proxy.")?;
    } else {
        url.set_host(Some(host))
            .map_err(|_| "Invalid system proxy.")?;
    }
    url.set_port(Some(port))
        .map_err(|()| "Invalid system proxy.")?;
    if let Some(user) = user {
        url.set_username(user)
            .map_err(|()| "Invalid system proxy.")?;
    }
    if let Some(password) = password {
        url.set_password(Some(password))
            .map_err(|()| "Invalid system proxy.")?;
    }
    Ok(url.to_string())
}

#[cfg(any(windows, test))]
pub(super) fn routes(url: &str, proxies: &str, bypass: &str) -> Result<Vec<String>, &'static str> {
    let url = url::Url::parse(url).map_err(|_| "Invalid proxy target.")?;
    let host = url
        .host_str()
        .ok_or("Invalid proxy target.")?
        .to_ascii_lowercase();
    if bypass
        .split([';', ' '])
        .filter(|v| !v.is_empty())
        .any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();
            (pattern == "<local>" && !host.contains('.') && !host.contains(':'))
                || wildcard(&pattern, &host)
        })
    {
        return Ok(vec!["direct://".to_owned()]);
    }
    let mut general = Vec::new();
    let mut exact = Vec::new();
    let mut socks = Vec::new();
    for entry in proxies.split([';', ' ']).filter(|v| !v.is_empty()) {
        if let Some((scheme, address)) = entry.split_once('=') {
            if !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ftp" | "socks"
            ) {
                return Err("Unsupported system proxy kind.");
            }
            if scheme.eq_ignore_ascii_case(url.scheme()) {
                exact.push(normalize("http", address));
            } else if scheme.eq_ignore_ascii_case("socks") {
                socks.push(normalize("socks4a", address));
            }
        } else {
            general.push(normalize("http", entry));
        }
    }
    let chosen = if !exact.is_empty() {
        exact
    } else if !general.is_empty() {
        general
    } else {
        socks
    };
    // Scheme maps without a rule for this scheme mean DIRECT; a wholly empty configured proxy is invalid.
    if chosen.is_empty() && proxies.contains('=') {
        return Ok(vec!["direct://".to_owned()]);
    }
    if chosen.is_empty() {
        return Err("Invalid system proxy.");
    }
    Ok(chosen)
}
#[cfg(any(windows, test))]
fn normalize(scheme: &str, address: &str) -> String {
    if address.contains("://") {
        address.to_owned()
    } else {
        format!("{scheme}://{address}")
    }
}
#[cfg(any(windows, test))]
fn wildcard(pattern: &str, value: &str) -> bool {
    let (mut p, mut v, mut star, mut retry) = (0, 0, None, 0);
    let (pats, vals) = (pattern.as_bytes(), value.as_bytes());
    while v < vals.len() {
        if p < pats.len() && pats[p] == vals[v] {
            p += 1;
            v += 1;
        } else if p < pats.len() && pats[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = v;
        } else if let Some(at) = star {
            p = at + 1;
            retry += 1;
            v = retry;
        } else {
            return false;
        }
    }
    while p < pats.len() && pats[p] == b'*' {
        p += 1;
    }
    p == pats.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_system_proxy_kinds_cannot_become_direct() {
        assert!(routes("https://public.test/", "socks5=proxy.test:1080", "").is_err());
        assert_eq!(
            routes("https://public.test/", "http=proxy.test:8080", "").unwrap(),
            ["direct://"]
        );
    }
    #[test]
    fn native_ipv6_proxy_hosts_are_preserved() {
        assert_eq!(
            endpoint("http", "::1", 8080, None, None).unwrap(),
            "http://[::1]:8080/"
        );
    }
    #[test]
    fn windows_scheme_map_bypass_and_auth_are_preserved() {
        assert_eq!(
            routes(
                "https://public.test/x",
                "http=a:1;https=https://u:pw@b:2;socks=c:3",
                "*.corp;<local>"
            )
            .unwrap(),
            ["https://u:pw@b:2"]
        );
        assert_eq!(
            routes("https://printer/", "a:1", "<local>").unwrap(),
            ["direct://"]
        );
        assert_eq!(
            routes("https://x.corp/", "a:1", "*.corp").unwrap(),
            ["direct://"]
        );
        assert_eq!(
            routes("http://public.test/", "socks=u:pw@c:3", "").unwrap(),
            ["socks4a://u:pw@c:3"]
        );
        assert_eq!(
            endpoint("socks5", "proxy.test", 1080, Some("u@ser"), Some("p:ss")).unwrap(),
            "socks5://u%40ser:p%3Ass@proxy.test:1080"
        );
    }
}
