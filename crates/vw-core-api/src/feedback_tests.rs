//! Feedback wire regression tests; core commands must accept review but never caller-supplied system data.

fn draft() -> crate::FeedbackDraft {
    crate::FeedbackDraft {
        kind: crate::FeedbackKind::Bug,
        title: "Tab disappeared".into(),
        description: "Open two tabs, close the second; both disappear.".into(),
        remarks: String::new(),
    }
}

#[tokio::test]
async fn feedback_delivery_sends_only_the_reviewed_payload_and_requires_a_storage_receipt() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let core = crate::offline_core::start("feedback-wire-redaction").await;
    let mut input = draft();
    let secret = ["gl", "pat-", "abcdefghijklmnopqrst"].concat();
    input.remarks = secret.clone();
    core.set_feedback_capture(|| async {
        vw_preview::encode(&vw_preview::canvas(40, 20, [128, 128, 128, 255]))
    });
    core.feedback(
        crate::Origin::Ui,
        crate::FeedbackRequest::Open {
            draft: input.clone(),
        },
    )
    .await
    .unwrap();
    core.feedback(
        crate::Origin::Ui,
        crate::FeedbackRequest::Screenshot { include: true },
    )
    .await
    .unwrap();
    let report = core
        .feedback(
            crate::Origin::Ui,
            crate::FeedbackRequest::Review { draft: input },
        )
        .await
        .unwrap()
        .review
        .unwrap();
    assert!(!report.draft.remarks.contains(&secret));
    for status in [201, 302, 202] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = serde_json::to_vec(&report).unwrap();
        let receipt_id = report.id.clone();
        let secret = secret.clone();
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
            assert!(!String::from_utf8_lossy(&bytes[header_end..]).contains(&secret));
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

/// PREDICT: the agent's schema listed remarks as required and the core refused a draft without it ("Supply kind,
/// title, description and remarks"), although the dialog marks Remarks optional; a draft without remarks now opens
/// with empty remarks and the schema requires only kind, title and description.
#[tokio::test]
async fn an_agent_draft_without_remarks_opens_like_the_dialog_allows() {
    let tools = vw_agent_tools::mcp::tool_list();
    let spec = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == vw_agent_tools::wire::tools::FEEDBACK_DRAFT)
        .unwrap();
    assert_eq!(
        spec["inputSchema"]["required"],
        serde_json::json!(["kind", "title", "description"])
    );
    let core = crate::offline_core::start("feedback-optional-remarks").await;
    let _ = core
        .shared
        .store
        .load(
            serde_json::json!({"session":{"id":"s"},"history":[],"turns":[]}),
            vw_session::LoadKind::New,
        )
        .unwrap();
    let offered = crate::feedback::offer(
        &core.shared,
        "s",
        serde_json::json!({"kind":"bug","title":"Tab disappeared",
            "description":"Open two tabs, close the second; both disappear."}),
    )
    .await;
    assert_eq!(offered.unwrap()["status"], "draftOffered");
    let view = core
        .feedback(crate::Origin::Ui, crate::FeedbackRequest::Read)
        .await
        .unwrap();
    let remarks = view.draft.unwrap().remarks;
    assert_eq!(remarks, "");
    core.shutdown().await;
}
