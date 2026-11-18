//! One browser transport budget across person pause and delayed helper replies; no native browser.
use super::super::{output_tests, test_support};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn pause_and_delayed_helper_share_the_browser_step_deadline() {
    let core = output_tests::core("browser-step-time-budget").await;
    tokio::time::pause();
    let shared = core.shared.clone();
    shared
        .paused_tools
        .pause("s", crate::paused_tools::BROWSER, "Chrome");
    let resume = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(39)).await;
        resume
            .paused_tools
            .resume("s", Some(crate::paused_tools::BROWSER));
    });
    shared
        .chrome_control
        .owner
        .answering_after(Duration::from_secs(30), |tool, args| {
            if tool == "chrome_status" {
                output_tests::status(args)
            } else {
                output_tests::page()
            }
        });
    let start = tokio::time::Instant::now();
    let answer = super::call(
        &shared,
        "s",
        "chrome_execute",
        json!({"sessionId":"t","action":"page.read","args":{}}),
    )
    .await;
    let elapsed = start.elapsed();
    println!("BROWSER_STEP elapsed={elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(46),
        "pause plus helper took {elapsed:?}: {answer:?}"
    );
    assert!(
        !answer
            .as_ref()
            .is_ok_and(|a| a["vwMcpResult"]["structuredContent"]["status"] == "completed"),
        "an unconfirmed read cannot be reported as completed: {answer:?}"
    );
    match &answer {
        Ok(value) => assert_eq!(
            value["vwMcpResult"]["structuredContent"]["error"]["code"], "BROWSER_STEP_TIMEOUT",
            "a spent deadline is not a lost connection: {answer:?}"
        ),
        Err(reason) => assert!(reason.contains("reached its time limit"), "{reason}"),
    }
    core.shutdown().await;
}

#[tokio::test]
async fn a_second_chats_read_does_not_spend_the_first_chats_pause_budget() {
    let core = output_tests::core("independent-browser-step-budgets").await;
    test_support::chat(&core.shared, "other");
    core.shared
        .chrome_control
        .tasks
        .lock()
        .unwrap()
        .insert("other-tab".into(), "other".into());
    test_support::answering(&core.shared, |tool, args| {
        if tool == "chrome_status" {
            return output_tests::status(args);
        }
        let mut page = output_tests::page();
        page["structuredContent"]["sessionId"] = args["sessionId"].clone();
        page
    });
    core.shared
        .paused_tools
        .pause("s", crate::paused_tools::BROWSER, "Chrome");
    let paused = core.shared.clone();
    let waiting = tokio::spawn(async move {
        super::call(
            &paused,
            "s",
            "chrome_execute",
            json!({"sessionId":"t","action":"page.read","args":{}}),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !core.shared.paused_tools.view("s").paused[0].waiting {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let answer = super::call(
        &core.shared,
        "other",
        "chrome_execute",
        json!({"sessionId":"other-tab","action":"page.read","args":{}}),
    )
    .await
    .unwrap();
    assert_eq!(
        answer["vwMcpResult"]["structuredContent"]["status"],
        "completed"
    );
    assert!(
        !waiting.is_finished(),
        "the other chat cannot resume this held call"
    );
    core.shared
        .paused_tools
        .resume("s", Some(crate::paused_tools::BROWSER));
    assert!(waiting.await.unwrap().is_ok());
    core.shutdown().await;
}
