//! Backend media fetch policy, independent of browser navigation permissions.
//! Called by capture admission; preserves the bounded port, address and per-page restrictions.
use crate::policy::{Destination, PageClass, UrlPolicy, UrlRefusal};
use url::{Host, Url};
use vw_netguard::{
    CANARY_SUFFIX,
    ip::{IpClass, classify},
};
fn short(text: &str) -> String {
    text.chars().take(80).collect()
}
impl UrlPolicy {
    pub(crate) fn fetch_classify(&self, url: &Url) -> Result<Destination, UrlRefusal> {
        if url.as_str() == "about:blank" {
            return Ok(Destination::Blank);
        }
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(UrlRefusal::Scheme(short(scheme)));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(UrlRefusal::UserInfo);
        }
        let port = url.port_or_known_default().unwrap_or(80);
        let loopback = match url.host() {
            Some(Host::Ipv4(v4)) => {
                if v4.octets() == [127, 0, 0, 1] {
                    true
                } else {
                    return literal(&v4.to_string(), classify(v4.into()));
                }
            }
            Some(Host::Ipv6(v6)) => {
                if v6.is_loopback() {
                    true
                } else {
                    return literal(&format!("[{v6}]"), classify(v6.into()));
                }
            }
            Some(Host::Domain(name)) => {
                let lower = name.to_ascii_lowercase();
                if lower == "localhost" {
                    true
                } else if lower.ends_with(CANARY_SUFFIX) && scheme == "http" {
                    return Ok(Destination::Canary);
                } else if lower == "localhost."
                    || lower.trim_end_matches('.').ends_with(".localhost")
                {
                    return Err(UrlRefusal::ReservedName(short(&lower)));
                } else {
                    return Ok(Destination::Public);
                }
            }
            None => return Err(UrlRefusal::NotAUrl(short(url.as_str()))),
        };
        debug_assert!(loopback);
        if scheme != "http" {
            return Err(UrlRefusal::LoopbackHttps);
        }
        if self.is_app_origin_port(port) {
            return Err(UrlRefusal::AppOrigin(port));
        }
        Ok(Destination::Loopback { port })
    }

    fn is_app_origin_port(&self, port: u16) -> bool {
        // Every loopback spelling reaches the same server, so the port alone decides once the dev origin is
        // itself on loopback (it always is: `devUrl` = http://127.0.0.1:1420).
        self.dev_origin.as_ref().is_some_and(|dev| {
            let dev_on_loopback = match dev.host() {
                Some(Host::Ipv4(v)) => v.is_loopback(),
                Some(Host::Ipv6(v)) => v.is_loopback(),
                Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
                None => false,
            };
            dev_on_loopback && dev.port_or_known_default() == Some(port)
        })
    }

    pub(crate) fn fetch_navigation(
        &self,
        url: &Url,
        allowed: &[u16],
        class: PageClass,
        lan_host: Option<&str>,
    ) -> Result<(), UrlRefusal> {
        let destination = self.fetch_classify(url)?;
        let same_lan_host =
            class == PageClass::Lan && lan_host.is_some_and(|h| url.host_str() == Some(h));
        match destination {
            Destination::Blank => return Ok(()),
            Destination::Canary => {
                return Err(UrlRefusal::ReservedName(short(
                    url.host_str().unwrap_or(""),
                )));
            }
            Destination::Loopback { port } if !allowed.contains(&port) => {
                return Err(UrlRefusal::Unattributed(port));
            }
            Destination::Lan | Destination::Public if same_lan_host => return Ok(()),
            Destination::Lan => return Err(UrlRefusal::Lan(short(url.host_str().unwrap_or("")))),
            _ => {}
        }
        if destination.class() == class {
            Ok(())
        } else {
            Err(UrlRefusal::OtherClass(short(url.as_str())))
        }
    }
}
fn literal(text: &str, class: IpClass) -> Result<Destination, UrlRefusal> {
    match class {
        IpClass::Public => Ok(Destination::Public),
        // Backend fetches keep the existing link-local and reserved-address refusals.
        IpClass::Private | IpClass::SharedCgnat => Ok(Destination::Lan),
        _ => Err(UrlRefusal::Host(short(text), class.describe())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backend_fetch_still_refuses_private_addresses_from_a_web_page() {
        let policy = UrlPolicy::default();
        for text in [
            "http://172.16.1.2/image.png",
            "http://10.1.2.3/",
            "http://169.254.169.254/",
        ] {
            assert!(
                policy
                    .fetch_navigation(&Url::parse(text).unwrap(), &[], PageClass::Web, None)
                    .is_err(),
                "{text}"
            );
        }
        assert!(
            policy
                .fetch_navigation(
                    &Url::parse("https://93.184.216.34/image.png").unwrap(),
                    &[],
                    PageClass::Web,
                    None
                )
                .is_ok()
        );
    }
}
