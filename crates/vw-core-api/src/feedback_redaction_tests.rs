//! App-only synthetic redaction cases; verifies review, outbound serialization and bounded redaction work.
use crate::{FeedbackDraft, FeedbackKind, FeedbackRequest, Origin};

#[tokio::test]
async fn feedback_redaction_cases_reach_review_and_wire() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/public/fixtures/feedback-redaction.json"
    ))
    .unwrap();
    let core = crate::offline_core::start("feedback-redaction").await;
    for row in fixture["cases"].as_array().unwrap() {
        // Provider formats are assembled only in memory: committed fixtures contain harmless markers.
        let marker = row["credentialParts"].as_array().map(|parts| {
            parts
                .iter()
                .map(|part| part.as_str().unwrap())
                .collect::<String>()
        });
        let input = row["input"].as_str().unwrap().replace(
            "<synthetic-credential>",
            marker.as_deref().unwrap_or("<synthetic-credential>"),
        );
        let expected = row["expected"].as_str().unwrap().replace(
            "<synthetic-credential>",
            marker.as_deref().unwrap_or("<synthetic-credential>"),
        );
        assert_eq!(
            crate::feedback_redact::text(&input, &[]),
            expected,
            "{}",
            row["name"]
        );
        let review = core
            .feedback(
                Origin::Ui,
                FeedbackRequest::Review {
                    draft: FeedbackDraft {
                        kind: FeedbackKind::Bug,
                        title: "Tab disappears".into(),
                        description: "Close the second tab.".into(),
                        remarks: input,
                    },
                },
            )
            .await
            .unwrap()
            .review
            .unwrap();
        assert_eq!(review.draft.remarks, expected, "{}", row["name"]);
        let wire: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&review).unwrap()).unwrap();
        assert_eq!(wire["draft"]["remarks"], expected, "{}", row["name"]);
    }
    core.shutdown().await;
}

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

#[tokio::test]
async fn feedback_redaction_accepts_the_field_limit_and_rejects_control_characters() {
    let core = crate::offline_core::start("feedback-redaction-limit").await;
    for (description, accepted) in [
        ("a-".repeat(4000), true),
        ("a.".repeat(4000), true),
        ("a.".repeat(4001), false),
        ("key\u{0085}=abcdefghijklmnopqrst".into(), false),
    ] {
        let result = core
            .feedback(
                Origin::Ui,
                FeedbackRequest::Review {
                    draft: FeedbackDraft {
                        kind: FeedbackKind::Bug,
                        title: "Tab disappears".into(),
                        description,
                        remarks: String::new(),
                    },
                },
            )
            .await;
        assert_eq!(result.is_ok(), accepted);
    }
    core.shutdown().await;
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
