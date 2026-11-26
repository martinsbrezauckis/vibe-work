//! The first byte of a macOS `WebKit` CONNECT payload distinguishes HTTP from TLS without consuming it.
//! This is used only for OS PAC lookup of browser tunnels; manual proxy matching retains its old URL.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Protocol {
    Http,
    Https,
}

pub(crate) fn classify(first: u8) -> Result<Protocol, ()> {
    match first {
        0x16 => Ok(Protocol::Https),
        b'A'..=b'Z' => Ok(Protocol::Http),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{Protocol, classify};

    #[test]
    fn uppercase_http_method_initials_and_tls_client_hello_are_distinct() {
        for first in *b"GPHDOCT" {
            assert_eq!(classify(first), Ok(Protocol::Http), "byte {first:#x}");
        }
        assert_eq!(classify(0x16), Ok(Protocol::Https));
    }

    #[test]
    fn unknown_protocol_bytes_fail_closed() {
        for first in [0, b'g', b' ', 0x15, 0x17, 0xff] {
            assert_eq!(classify(first), Err(()), "byte {first:#x}");
        }
    }
}
