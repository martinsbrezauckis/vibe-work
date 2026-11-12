//! App-only synthetic redaction cases; verifies review, outbound serialization and bounded redaction work.

#[test]
fn feedback_redaction_adversarial_scaling() {
    // Each embedded rule is a linear regex search for these zero/one-match adversarial inputs.
    // Count rule applications and admitted bytes, so repeated scans cannot hide behind a fast CPU.
    const PASSES: usize = 20;
    for chunk in ["a-", "a."] {
        for bytes in [8000, 16000, 32000, 65536] {
            let input = chunk.repeat(bytes / 2);
            let mut calls = 0;
            let mut work = 0;
            let result =
                crate::feedback_redact::text_with_work(&input, &[], |pattern, admitted| {
                    calls += 1;
                    work += admitted.len();
                    // Rust regex bounds a search by pattern size × input length. With zero/one
                    // match, replace_all needs at most two searches; repeated suffix searches
                    // cannot turn this adversarial family into quadratic replacement work.
                    assert!(
                        pattern.find_iter(admitted).take(2).count() <= 1,
                        "multiple suffix searches for {}",
                        pattern.as_str()
                    );
                });
            assert_eq!(
                result,
                if chunk == "a-" {
                    "[redacted key]"
                } else {
                    &input
                }
            );
            println!(
                "redaction {chunk} {bytes} bytes: {calls} rule applications, {work} input bytes"
            );
            assert_eq!(calls, PASSES);
            assert!(
                work <= PASSES * (bytes + "[redacted key]".len()),
                "repeated scans: {work}"
            );
        }
    }
}

#[test]
fn feedback_default_endpoint_uses_the_owned_host() {
    if option_env!("VW_FEEDBACK_URL").is_none() {
        assert_eq!(
            crate::feedback_transport::endpoint().unwrap().as_str(),
            "https://feedback.u1c.lv/v1/reports"
        );
    }
}
