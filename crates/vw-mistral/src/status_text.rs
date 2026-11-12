//! Fixed shared HTTP error wording for OCR, speech, Work and the key test.
//! Called by their typed Display implementations; response bodies and credentials never enter these texts.

pub(crate) const KEY_REJECTED: &str =
    "Mistral did not accept the API key (401). Check it in Settings.";
pub(crate) const RATE_LIMITED: &str =
    "Mistral is rate-limiting this key right now (429). Try again in a minute.";
pub(crate) const MALFORMED_STORED_KEY: &str =
    "The stored key contains characters a key cannot have.";

pub(crate) fn service_problem(status: u16, service: &str) -> String {
    format!("{service} had a problem ({status}). Try again later.")
}

pub(crate) fn unexpected(status: u16) -> String {
    format!("Mistral answered with an unexpected status ({status}).")
}
