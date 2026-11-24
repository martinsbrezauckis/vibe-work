use super::{read_helper_stderr, read_refusal};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const DEADLINE: &str = "Firefox BiDi deadline: action=stop method=session.end id=7 waited_ms=50 events=2 controls=1 proxy=0";
const LATE_REPLY: &str = "Firefox BiDi late reply: method=session.end id=7 after_ms=75";

#[tokio::test]
async fn stderr_reader_stays_alive_past_four_kib() {
    let (reader, mut writer) = tokio::io::duplex(64);
    let task = tokio::spawn(read_refusal(reader));
    tokio::time::timeout(Duration::from_secs(2), async {
        writer.write_all(&vec![b'x'; 5120]).await.unwrap();
        writer.write_all(b"still alive\n").await.unwrap();
        assert!(!task.is_finished(), "reader must wait for EOF");
        writer.shutdown().await.unwrap();
        assert_eq!(task.await.unwrap(), None);
    })
    .await
    .expect("stderr drain must not block the helper");
}

#[tokio::test]
async fn stderr_refusal_is_returned_only_at_eof() {
    let (reader, mut writer) = tokio::io::duplex(64);
    let task = tokio::spawn(read_refusal(reader));
    tokio::time::timeout(Duration::from_secs(2), async {
        writer
            .write_all(b"OWNER_APPROVAL_REQUIRED\n")
            .await
            .unwrap();
        writer.write_all(&vec![b'x'; 5120]).await.unwrap();
        assert!(!task.is_finished());
        writer.shutdown().await.unwrap();
        assert_eq!(task.await.unwrap(), Some("OWNER_APPROVAL_REQUIRED"));
    })
    .await
    .expect("reader must drain before returning refusal");
}

#[tokio::test]
async fn stderr_refusal_search_is_limited_to_first_four_kib() {
    let mut input = vec![b'x'; 4096];
    input.extend_from_slice(b"OWNER_CONFLICT\n");
    assert_eq!(read_refusal(input.as_slice()).await, None);
    assert_eq!(
        read_refusal(b"OWNER_IDENTITY_MISMATCH\n".as_slice()).await,
        Some("OWNER_IDENTITY_MISMATCH")
    );
}

#[tokio::test]
async fn stderr_only_forwards_complete_bounded_ascii_deadlines() {
    let mut input = vec![b'x'; 5120];
    input.push(b'\n');
    input.extend_from_slice(format!("{DEADLINE}\n").as_bytes());
    input.extend_from_slice(b"secret unrelated text\nnot Firefox BiDi deadline: action=stop\n");
    input.extend_from_slice(format!("Firefox BiDi deadline: {}\n", "x".repeat(700)).as_bytes());
    input.extend_from_slice("Firefox BiDi deadline: é\n".as_bytes());
    input.extend_from_slice(b"Firefox BiDi deadline: tab\t\nFirefox BiDi deadline: control\x1b\n");
    let boundary = format!(
        "Firefox BiDi deadline: {}",
        "x".repeat(600 - "Firefox BiDi deadline: ".len())
    );
    input.extend_from_slice(format!("{boundary}\n").as_bytes());
    input.extend_from_slice(DEADLINE.as_bytes()); // incomplete line at EOF
    let mut lines = Vec::new();
    assert_eq!(
        read_helper_stderr(input.as_slice(), |line| lines.push(line.to_owned())).await,
        None
    );
    assert_eq!(lines, [DEADLINE, boundary.as_str()]);
}

#[tokio::test]
async fn stderr_forwards_late_reply_after_five_kib() {
    let mut input = vec![b'x'; 5120];
    input.extend_from_slice(format!("\n{LATE_REPLY}\n").as_bytes());
    let mut lines = Vec::new();
    read_helper_stderr(input.as_slice(), |line| lines.push(line.to_owned())).await;
    assert_eq!(lines, [LATE_REPLY]);
}

#[tokio::test]
async fn stderr_rejects_near_miss_diagnostic_prefixes() {
    let input = b"Firefox BiDi late reply\nFirefox BiDi late reply:method=session.end\nFirefox BiDi other: x\n";
    let mut lines = Vec::new();
    read_helper_stderr(input.as_slice(), |line| lines.push(line.to_owned())).await;
    assert_eq!(lines, [] as [String; 0]);
}

#[tokio::test]
async fn stderr_shares_diagnostic_cap_between_both_prefixes() {
    for (first, second) in [(DEADLINE, LATE_REPLY), (LATE_REPLY, DEADLINE)] {
        let mut input = format!("{first}\n").repeat(199).into_bytes();
        input.extend_from_slice(format!("{second}\n{first}\n{second}\n").as_bytes());
        let mut lines = Vec::new();
        read_helper_stderr(input.as_slice(), |line| lines.push(line.to_owned())).await;
        assert_eq!(lines.len(), 201);
        assert!(lines[..199].iter().all(|line| line == first));
        assert_eq!(lines[199], second);
        assert_eq!(lines[200], "further helper diagnostics suppressed");
    }
}

#[tokio::test]
async fn stderr_caps_diagnostics_but_keeps_draining() {
    let mut input = format!("{DEADLINE}\n").repeat(205).into_bytes();
    input.extend_from_slice(&vec![b'x'; 8192]);
    let mut lines = Vec::new();
    read_helper_stderr(input.as_slice(), |line| lines.push(line.to_owned())).await;
    assert_eq!(lines.len(), 201);
    assert!(lines[..200].iter().all(|line| line == DEADLINE));
    assert_eq!(lines[200], "further helper diagnostics suppressed");
}
