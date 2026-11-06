//! App-only synthetic redaction cases; verifies review, outbound serialization and bounded-input performance.
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
    use std::time::{Duration, Instant};
    // Lazy compilation is a one-time cost; measure repeated redaction with the embedded rules loaded.
    crate::feedback_redact::text("Warm the rules", &[]);
    for chunk in ["a-", "a."] {
        let mut baseline = Duration::ZERO;
        for bytes in [8000_u32, 16000, 32000, 65536] {
            // 8000 is the description limit; larger inputs probe the redactor, not report admission.
            let input = chunk.repeat(usize::try_from(bytes / 2).unwrap());
            let mut samples = Vec::new();
            for _ in 0..9 {
                let start = Instant::now();
                let result = crate::feedback_redact::text(std::hint::black_box(&input), &[]);
                samples.push(start.elapsed());
                assert_eq!(
                    result,
                    if chunk == "a-" {
                        "[redacted key]"
                    } else {
                        &input
                    }
                );
            }
            samples.sort();
            let median = samples[4];
            println!("redaction {chunk} {bytes} bytes: {median:?} median (9 runs)");
            if bytes == 8000 {
                baseline = median;
            } else {
                // Allow scheduling noise, but reject quadratic growth over this 8x size range.
                assert!(median < baseline * (2 * bytes / 8000) + Duration::from_millis(1));
            }
            if !cfg!(debug_assertions) {
                assert!(
                    median < Duration::from_millis(20),
                    "{bytes} bytes took {median:?}"
                );
            }
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
