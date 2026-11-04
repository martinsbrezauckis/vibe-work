//! Feedback wire regression tests; core commands must accept review but never caller-supplied system data.
use crate::Command;
use serde_json::json;
#[test]
fn feedback_wire_exposes_review_without_editable_diagnostics() {
    let draft = json!({"kind":"bug","title":"Tab disappeared","description":"Open a tab, then close another.","remarks":""});
    let review = json!({"type":"feedback","request":{"op":"review","draft":draft}});
    assert!(serde_json::from_value::<Command>(review.clone()).is_ok());
    let mut tampered = review;
    tampered["request"]["system"] = json!({"os":"made up"});
    assert!(serde_json::from_value::<Command>(tampered).is_err());
}
#[test]
fn feedback_tool_can_only_offer_a_draft() {
    let tools = vw_agent_tools::wire::tools::SESSION_TOOLS;
    assert!(tools.contains(&"feedback_draft"));
    assert!(!tools.contains(&"feedback_send"));
}

fn draft() -> crate::FeedbackDraft {
    crate::FeedbackDraft {
        kind: crate::FeedbackKind::Bug,
        title: "Tab disappeared".into(),
        description: "Open two tabs, close the second; both disappear.".into(),
        remarks: String::new(),
    }
}
#[tokio::test]
async fn feedback_requires_review_and_rejects_automation_confirmation() {
    use crate::{FeedbackRequest as R, Origin};
    let core = crate::offline_core::start("feedback-review").await;
    let offered = core
        .feedback(Origin::DebugApi, R::Open { draft: draft() })
        .await
        .unwrap();
    assert!(offered.review.is_none());
    assert!(!offered.share_system);
    assert!(
        core.feedback(
            Origin::Ui,
            R::Send {
                review_id: "invented".into()
            }
        )
        .await
        .is_err()
    );
    let reviewed = core
        .feedback(Origin::Ui, R::Review { draft: draft() })
        .await
        .unwrap();
    let report = reviewed.review.unwrap();
    assert!(report.system.is_none());
    let error = core
        .feedback(
            Origin::DebugApi,
            R::Send {
                review_id: report.id.clone(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, crate::ErrorCode::Denied);
    core.set_window_drive(|| true);
    assert_eq!(
        core.feedback(
            Origin::Ui,
            R::Send {
                review_id: report.id
            }
        )
        .await
        .unwrap_err()
        .code,
        crate::ErrorCode::Denied
    );
    assert!(
        core.feedback(Origin::Ui, R::Read)
            .await
            .unwrap()
            .review
            .is_some()
    );
    core.shutdown().await;
}
#[tokio::test]
async fn feedback_consent_is_optional_remembered_and_cannot_change_facts() {
    use crate::{FeedbackRequest as R, Origin};
    let dir = std::env::var_os("VW_TEST_ROOT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("feedback-prefs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), "{\"unrelated\":true}").unwrap();
    let core = crate::offline_core::start_with("feedback-prefs", |config, _| {
        config.secret_scan_data = Some(dir.clone());
    })
    .await;
    core.feedback(Origin::Ui, R::Open { draft: draft() })
        .await
        .unwrap();
    let shown = core
        .feedback(Origin::Ui, R::Share { share_system: true })
        .await
        .unwrap();
    let reviewed = core
        .feedback(Origin::Ui, R::Review { draft: draft() })
        .await
        .unwrap()
        .review
        .unwrap();
    assert_eq!(reviewed.system, Some(shown.system));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["feedbackShareSystem"], true);
    assert_eq!(saved["unrelated"], true);
    assert!(!saved.to_string().contains("Tab disappeared"));
    core.shutdown().await;
    let core = crate::offline_core::start_with("feedback-prefs-restart", |config, _| {
        config.secret_scan_data = Some(dir.clone());
    })
    .await;
    assert!(
        core.feedback(Origin::Ui, R::Read)
            .await
            .unwrap()
            .share_system
    );
    core.feedback(
        Origin::Ui,
        R::Share {
            share_system: false,
        },
    )
    .await
    .unwrap();
    assert!(
        core.feedback(Origin::Ui, R::Review { draft: draft() })
            .await
            .unwrap()
            .review
            .unwrap()
            .system
            .is_none()
    );
    core.shutdown().await;
}
#[tokio::test]
async fn feedback_redacts_before_preview_and_does_not_overwrite_open_work() {
    use crate::{FeedbackRequest as R, Origin};
    let core = crate::offline_core::start("feedback-redaction").await;
    let mut text = draft();
    text.remarks = "api_key=abcdef1234567890 /home/alice/work/file C:\\Users\\Bob\\source token: deadbeef0123456789".into();
    let first = core
        .feedback(Origin::DebugApi, R::Open { draft: text })
        .await
        .unwrap();
    let shown = first.draft.unwrap().remarks;
    for value in ["abcdef1234567890", "alice", "Bob", "deadbeef0123456789"] {
        assert!(!shown.contains(value), "{shown}");
    }
    assert!(
        core.feedback(Origin::DebugApi, R::Open { draft: draft() })
            .await
            .is_err()
    );
    core.feedback(Origin::Ui, R::Close).await.unwrap();
    assert!(
        core.feedback(Origin::Ui, R::Read)
            .await
            .unwrap()
            .draft
            .is_none()
    );
    assert_eq!(
        crate::feedback_redact::text("ALICE reports a bug", &["alice".into()]),
        "[redacted] reports a bug"
    );
    core.shutdown().await;
}
#[test]
fn feedback_redaction_handles_quoted_keys_and_paths_with_spaces() {
    let source = r#"{"MISTRAL_API_KEY": "short-value", "password": "secret-value"} file="C:\Users\Example Person\source.txt""#;
    let redacted = crate::feedback_redact::text(source, &[]);
    for value in ["short-value", "secret-value", "Example", "Person"] {
        assert!(!redacted.contains(value), "{redacted}");
    }
}

#[tokio::test]
async fn feedback_delivery_sends_only_the_reviewed_payload_and_requires_a_storage_receipt() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let core = crate::offline_core::start("feedback-wire-redaction").await;
    let mut input = draft();
    let secret = "glpat-abcdefghijklmnopqrst";
    input.remarks = secret.into();
    let report = core
        .feedback(
            crate::Origin::Ui,
            crate::FeedbackRequest::Review { draft: input },
        )
        .await
        .unwrap()
        .review
        .unwrap();
    assert!(!report.draft.remarks.contains(secret));
    for status in [201, 302, 202] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = serde_json::to_vec(&report).unwrap();
        let receipt_id = report.id.clone();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 2048];
            let header_end = loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(i) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec())
                .unwrap()
                .to_ascii_lowercase();
            assert!(!headers.contains("authorization:"));
            assert!(!headers.contains("cookie:"));
            while bytes.len() - header_end < expected.len() {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
            }
            assert_eq!(&bytes[header_end..], expected);
            assert!(!String::from_utf8_lossy(&bytes[header_end..]).contains(secret));
            let body = serde_json::json!({"id":receipt_id,"stored":true}).to_string();
            let reply = format!(
                "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nContent-Type: application/json\r\nLocation: http://127.0.0.1:1/forbidden\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        let result = crate::feedback_transport::send(
            url::Url::parse(&format!("http://{address}/v1/reports")).unwrap(),
            &report,
        )
        .await;
        assert_eq!(result.is_ok(), status == 201);
        task.await.unwrap();
    }
    core.shutdown().await;
}
#[test]
fn feedback_redaction_masks_local_usernames_before_splitting_unquoted_paths() {
    let redacted = crate::feedback_redact::text(
        r"C:\Users\Example Person\project\file.txt failed",
        &["Example Person".into()],
    );
    assert!(!redacted.contains("Person"), "{redacted}");
}

#[test]
fn feedback_redaction_masks_complete_quoted_credentials_and_identities() {
    let redacted = crate::feedback_redact::text(
        r#"password="two word passphrase" username='Example Person'"#,
        &[],
    );
    for text in ["two", "word", "passphrase", "Example", "Person"] {
        assert!(!redacted.contains(text), "{redacted}");
    }
}

#[test]
fn feedback_redaction_masks_unquoted_home_paths_for_unknown_accounts() {
    let redacted = crate::feedback_redact::text(
        "C:\\Users\\Sample Person\\Specimen Directory\\file.txt\n/Users/Sample Person/Specimen Directory/file.txt\n/home/sample person/Specimen Directory/file.txt",
        &[],
    );
    for text in ["Person", "person", "Specimen", "file.txt"] {
        assert!(!redacted.contains(text), "{redacted}");
    }
}
