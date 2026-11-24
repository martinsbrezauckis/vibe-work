//! Regression for nested blob URL origins used by the native request filter.
//! Runs each origin call in a child test process because stack overflow aborts Rust.

use std::process::Command;

fn nested_blob_url() -> String {
    format!("{}https://example.com/", "blob:".repeat(10_000))
}

// These expectations report a broken child-test harness as a named test failure.
#[allow(clippy::expect_used)]
fn assert_child_survives(test_name: &str) -> bool {
    if std::env::var_os("VW_URL17_ORIGIN_CHILD").is_some() {
        return true;
    }
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env("VW_URL17_ORIGIN_CHILD", "1")
        .output()
        .expect("spawn origin test child");
    assert!(
        output.status.success(),
        "{test_name} child status: {}; stderr: {}; stdout: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    false
}

#[test]
fn nested_blob_origin_stays_within_default_thread_stack() {
    if !assert_child_survives("nested_blob_origin_stays_within_default_thread_stack") {
        return;
    }
    std::thread::spawn(|| {
        let url = url::Url::parse(&nested_blob_url()).expect("nested blob parses");
        assert!(!url.origin().is_tuple(), "nested blob has opaque origin");
    })
    .join()
    .expect("origin thread survives");
}

#[test]
fn native_request_matcher_survives_nested_blob_url() {
    if !assert_child_survives("native_request_matcher_survives_nested_blob_url") {
        return;
    }
    let request = std::thread::spawn(|| {
        let matcher = vw_preview::shields::matcher::Matcher::new("[]").expect("empty rules");
        assert!(!matcher.blocks(&nested_blob_url(), "script", "https://example.com/", false));
        assert!(!matcher.blocks("", "script", "https://example.com/", false));
    });
    let page = std::thread::spawn(|| {
        let matcher = vw_preview::shields::matcher::Matcher::new("[]").expect("empty rules");
        assert!(!matcher.blocks(
            "https://example.com/asset.js",
            "script",
            &nested_blob_url(),
            false
        ));
        assert!(!matcher.blocks("https://example.com/asset.js", "script", "", false));
    });
    request.join().expect("request URI matcher thread survives");
    page.join().expect("page URL matcher thread survives");
}
