//! A failure of the OS credential store reads as a plain reason that names the store; its own text never reaches the
//! person (Linux: "SS error: result not returned from SS API" when the Secret Service has no default collection).
//!
//! PREDICT (written before the change): `store_failure` does not exist and the store's text was wrapped as it is, so
//! the setup result showed "this device cannot be remembered: Couldn't access platform storage: SS error: …".
use super::*;

#[test]
fn a_store_failure_names_the_store_and_drops_its_own_text() {
    for raw in [
        "Couldn't access platform storage: SS error: result not returned from SS API",
        "Platform failure: org.freedesktop.DBus.Error.ServiceUnknown",
        "SS error: prompt dismissed",
    ] {
        let error = store_failure(raw.to_owned());
        let text = error.to_string();
        assert_eq!(
            text,
            format!("this device cannot be remembered: {STORE_UNAVAILABLE}")
        );
        assert!(
            !text.contains("SS error") && !text.contains("Platform failure"),
            "{text}"
        );
    }
    #[cfg(target_os = "linux")]
    assert!(STORE_UNAVAILABLE.contains("Secret Service"));
}

#[test]
fn a_store_that_did_not_answer_keeps_its_plain_reason() {
    assert_eq!(
        store_failure(NO_ANSWER.to_owned()).to_string(),
        format!("this device cannot be remembered: {NO_ANSWER}")
    );
}
