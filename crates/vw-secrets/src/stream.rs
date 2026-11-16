//! [`StreamScrub`]: [`crate::scrub`] for an answer that arrives in parts (an HTTP body, an event stream), so a value
//! split across two parts is withheld too.
//!
//! Each part is joined to the tail kept from the part before, scrubbed, and passed on except for the longest end that
//! could still be the start of a registered value's form: only that end waits for the next part (or the end of the
//! answer). A part whose end cannot start a value is passed on whole and at once, so a streamed event without a
//! value is never delayed.
//!
//! A scrub made [`StreamScrub::with_value`] also withholds that one value in every form at any length (the registry
//! skips values shorter than `MIN_LEN`, which would match ordinary text in every answer it checks).
//!
//! Depends on: the registry ([`crate::scrub`], [`crate::pending_prefix`]). Called by: `vw-keyproxy` (the MCP
//! forwarder's answers).

use zeroize::Zeroizing;

/// The scrubbing state of one answer: the registry's values, and the one value of [`StreamScrub::with_value`].
#[derive(Default)]
pub struct StreamScrub {
    tail: Vec<u8>,
    /// The marker and every form ([`crate::exact_forms`], any length) of the answer's own value.
    own: Option<(String, Vec<Zeroizing<Vec<u8>>>)>,
}

impl std::fmt::Debug for StreamScrub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StreamScrub(<redacted>)")
    }
}

impl StreamScrub {
    /// A scrub that also withholds `value` as `kind` at any length, the registry's minimum (`MIN_LEN`) aside:
    /// the caller handed this value to the answer's sender, so the answer is searched for it whatever its size.
    #[must_use]
    pub fn with_value(kind: crate::SecretKind, value: &str) -> Self {
        let forms = crate::exact_forms(value)
            .into_iter()
            .map(Zeroizing::new)
            .collect();
        Self {
            tail: Vec::new(),
            own: Some((crate::marker(kind), forms)),
        }
    }

    /// The bytes of `part` (after the kept tail) that can be passed on now, scrubbed.
    pub fn push(&mut self, part: &[u8]) -> Vec<u8> {
        let mut joined = std::mem::take(&mut self.tail);
        joined.extend_from_slice(part);
        let mut out = crate::scrub(&joined).map_or(joined, |(bytes, _)| bytes);
        if let Some((marker, forms)) = &self.own {
            for form in forms {
                if memchr::memmem::find(&out, form).is_some() {
                    out = crate::replace(&out, form, marker.as_bytes());
                }
            }
        }
        // Measured on the final bytes, after every replacement.
        let own = self.own.iter().flat_map(|(_, forms)| forms.iter());
        let keep = crate::pending_prefix(&out).max(crate::pending_in(&out, own));
        self.tail = out.split_off(out.len() - keep);
        out
    }

    /// The kept tail at the end of the answer (already scrubbed: it holds no whole value).
    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.tail)
    }
}

#[cfg(test)]
mod tests {
    use super::StreamScrub;

    const VALUE: &str = "stream-scrub-fake-value-91c2";

    fn run(parts: &[&[u8]]) -> (Vec<Vec<u8>>, Vec<u8>) {
        let mut scrub = StreamScrub::default();
        let passed: Vec<Vec<u8>> = parts.iter().map(|part| scrub.push(part)).collect();
        let mut all: Vec<u8> = passed.concat();
        all.extend(scrub.finish());
        (passed, all)
    }

    #[test]
    fn a_value_split_at_any_byte_is_withheld() {
        crate::register(crate::SecretKind::VaultValue, VALUE);
        let text = format!("data: {{\"text\":\"got {VALUE}\"}}\n\n");
        let marker = crate::marker(crate::SecretKind::VaultValue);
        for cut in 0..=text.len() {
            let (head, tail) = text.as_bytes().split_at(cut);
            let (_, all) = run(&[head, tail]);
            let all = String::from_utf8(all).unwrap();
            assert!(!all.contains(VALUE), "cut {cut}: {all}");
            assert_eq!(
                all,
                format!("data: {{\"text\":\"got {marker}\"}}\n\n"),
                "cut {cut}"
            );
        }
        // Three parts, the value's middle alone in the second.
        let (_, all) = run(&[b"x stream-scrub-", b"fake-val", b"ue-91c2 y"]);
        assert_eq!(String::from_utf8(all).unwrap(), format!("x {marker} y"));
    }

    #[test]
    fn the_own_value_is_withheld_at_any_length_and_split() {
        // Shorter than MIN_LEN: the registry ignores it, the answer's own scrub does not.
        let short = "k9Qz";
        crate::register(crate::SecretKind::VaultValue, short);
        let marker = crate::marker(crate::SecretKind::VaultValue);
        let (_, plain) = run(&[b"a k9", b"Qz b"]);
        assert_eq!(plain, b"a k9Qz b");
        let mut scrub = StreamScrub::with_value(crate::SecretKind::VaultValue, short);
        let mut all = scrub.push(b"a k9");
        assert_eq!(all, b"a ", "a start of the value waits");
        all.extend(scrub.push(b"Qz b k9Qz"));
        all.extend(scrub.finish());
        assert_eq!(
            String::from_utf8(all).unwrap(),
            format!("a {marker} b {marker}")
        );
        assert!(crate::holds(b"x-k9Qz", short));
        assert!(!format!("{scrub:?}").contains(short));
    }

    #[test]
    fn the_kept_tail_is_measured_after_the_own_value_is_replaced() {
        // The own value ends like the start of a registered value's hex form: before its replacement that end
        // would be kept, longer than what is left once the value is withheld.
        crate::register(crate::SecretKind::VaultValue, VALUE);
        let hex = VALUE.bytes().fold(String::new(), |mut hex, b| {
            let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{b:02x}"));
            hex
        });
        let own = format!("own-{}", &hex[..hex.len() - 1]);
        let mut scrub = StreamScrub::with_value(crate::SecretKind::Typed, &own);
        let mut all = scrub.push(own.as_bytes());
        all.extend(scrub.finish());
        assert_eq!(all, crate::marker(crate::SecretKind::Typed).as_bytes());
    }

    #[test]
    fn a_part_that_cannot_start_a_value_passes_at_once() {
        crate::register(crate::SecretKind::VaultValue, VALUE);
        let (passed, _) = run(&[b"data: first\n\n", b"data: second\n\n"]);
        assert_eq!(passed[0], b"data: first\n\n");
        assert_eq!(passed[1], b"data: second\n\n");
        // A start of the value waits; text that only looks like one is passed on with the next part.
        let (passed, all) = run(&[b"a stream-scr", b"oll b"]);
        assert_eq!(passed[0], b"a ");
        assert_eq!(all, b"a stream-scroll b");
    }
}
