//! Per-listener ingress capabilities. Used by guard/serve, independent of upstream proxy sign-in.
//! OS randomness is mandatory; credentials are never logged or included in Debug output.
//! Depends on: OS randomness and base64.
//! Called by: guard listener creation and proxy admission.

use base64::Engine as _;
use std::fmt::Write as _;
use std::net::SocketAddr;

pub(crate) struct Credential {
    password: String,
    basic: String,
}

impl Credential {
    pub(crate) fn new() -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|_| std::io::Error::other("proxy credential randomness unavailable"))?;
        let password = bytes
            .iter()
            .fold(String::with_capacity(64), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            });
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
        supplied
            .bytes()
            .zip(self.basic.bytes())
            .fold(0, |diff, (a, b)| diff | (a ^ b))
            == 0
    }
}
