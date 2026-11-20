//! Per-listener ingress capabilities. Used by guard/serve, independent of upstream proxy sign-in.
//! OS randomness is mandatory; credentials are never logged or included in Debug output.
//! Depends on: OS randomness, base64, hex and subtle (constant-time comparison).
//! Called by: guard listener creation and proxy admission.

use base64::Engine as _;
use std::net::SocketAddr;
use subtle::ConstantTimeEq as _;

pub(crate) struct Credential {
    password: String,
    basic: String,
}

impl Credential {
    pub(crate) fn new() -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|_| std::io::Error::other("proxy credential randomness unavailable"))?;
        let password = hex::encode(bytes);
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("vw:{password}"));
        Ok(Self { password, basic })
    }

    pub(crate) fn url(&self, address: SocketAddr) -> String {
        format!("http://vw:{}@{address}", self.password)
    }

    pub(crate) fn accepts(&self, head: &str) -> bool {
        let mut values = head.split("\r\n").skip(1).filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("proxy-authorization")
                .then_some(value.trim())
        });
        let Some(value) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let Some((scheme, supplied)) = value.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("basic") || supplied.len() != self.basic.len() {
            return false;
        }
        supplied.as_bytes().ct_eq(self.basic.as_bytes()).into()
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    fn head(lines: &[&str]) -> String {
        let mut head = String::from("CONNECT example.com:443 HTTP/1.1");
        for line in lines {
            head.push_str("\r\n");
            head.push_str(line);
        }
        head
    }

    #[test]
    fn the_password_is_64_lowercase_hex_digits_in_the_proxy_url() {
        let credential = super::Credential::new().unwrap();
        assert_eq!(credential.password.len(), 64);
        assert!(
            credential
                .password
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        let url = credential.url(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));
        assert_eq!(
            url,
            format!("http://vw:{}@127.0.0.1:8080", credential.password)
        );
    }

    #[test]
    fn accepts_exactly_one_basic_header_with_the_credential() {
        let credential = super::Credential::new().unwrap();
        let basic = credential.basic.clone();
        assert!(credential.accepts(&head(&[&format!("Proxy-Authorization: Basic {basic}")])));
        assert!(credential.accepts(&head(&[&format!("proxy-authorization:  basic {basic} ")])));
        assert!(!credential.accepts(&head(&[])), "absent");
        let other =
            base64::engine::general_purpose::STANDARD.encode(format!("vw:{}", "0".repeat(64)));
        assert_eq!(other.len(), basic.len());
        assert!(
            !credential.accepts(&head(&[&format!("Proxy-Authorization: Basic {other}")])),
            "same length, other value"
        );
        assert!(
            !credential.accepts(&head(&[&format!("Proxy-Authorization: Basic {basic}x")])),
            "longer"
        );
        assert!(
            !credential.accepts(&head(&[&format!("Proxy-Authorization: Bearer {basic}")])),
            "another scheme"
        );
        let one = format!("Proxy-Authorization: Basic {basic}");
        assert!(!credential.accepts(&head(&[&one, &one])), "two headers");
    }
}
