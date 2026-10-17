//! In-memory, zeroizing encoding variants. Uses standard encoders; called only by the scanner.

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use zeroize::Zeroizing;

use crate::{Encoding, Error, Secret};

const URL_RESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) struct Pattern {
    pub secret: usize,
    pub encoding: Encoding,
    pub bytes: Zeroizing<Vec<u8>>,
}

pub(crate) fn patterns(secrets: &[Secret]) -> Result<Vec<Pattern>, Error> {
    let mut patterns = Vec::new();
    for (index, secret) in secrets.iter().enumerate() {
        if secret.value.chars().count() < 8 {
            continue;
        }
        let first = patterns.len();
        let mut add = |encoding, bytes: Zeroizing<Vec<u8>>| {
            if !patterns[first..]
                .iter()
                .any(|pattern: &Pattern| *pattern.bytes == *bytes)
            {
                patterns.push(Pattern {
                    secret: index,
                    encoding,
                    bytes,
                });
            }
        };
        add(
            Encoding::AsIs,
            Zeroizing::new(secret.value.as_bytes().to_vec()),
        );
        // Reserve for the worst-case JSON expansion so secret bytes do not move on growth.
        let mut json = Zeroizing::new(Vec::with_capacity(secret.value.len() * 6 + 2));
        serde_json::to_writer(&mut *json, &*secret.value).map_err(|_| Error::Encoding)?;
        add(
            Encoding::JsonEscaped,
            Zeroizing::new(json[1..json.len() - 1].to_vec()),
        );
        for engine in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
            let mut encoded = Zeroizing::new(String::with_capacity(secret.value.len() * 2 + 4));
            engine.encode_string(secret.value.as_bytes(), &mut encoded);
            add(
                Encoding::Base64,
                Zeroizing::new(std::mem::take(&mut *encoded).into_bytes()),
            );
        }
        let encoded = utf8_percent_encode(&secret.value, URL_RESERVED);
        let mut url = Zeroizing::new(Vec::with_capacity(secret.value.len() * 3));
        for part in encoded {
            url.extend_from_slice(part.as_bytes());
        }
        add(Encoding::UrlEncoded, url);
        let form =
            form_urlencoded::Serializer::new(String::with_capacity(secret.value.len() * 3 + 6))
                .append_pair("value", &secret.value)
                .finish();
        let form = Zeroizing::new(form);
        add(
            Encoding::UrlEncoded,
            Zeroizing::new(form.as_bytes()[6..].to_vec()),
        );
    }
    Ok(patterns)
}
