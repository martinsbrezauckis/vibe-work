//! Chat teardown and retained-window ownership; tests the parent connection state without a browser.
use super::*;
#[test]
fn ending_a_chat_releases_its_tasks_and_deletion_forgets_its_window() {
    let chrome = ChromeControl::new(Path::new("."), ChromeControlConfig::default());
    chrome
        .tasks
        .lock()
        .unwrap()
        .extend([("a1".into(), "a".into()), ("b1".into(), "b".into())]);
    chrome
        .chat_profiles
        .lock()
        .unwrap()
        .insert("a".into(), "profile".into());
    chrome.end_chat("a", false);
    assert_eq!(chrome.tasks.lock().unwrap().len(), 1);
    assert!(chrome.tasks.lock().unwrap().contains_key("b1"));
    assert!(chrome.chat_profiles.lock().unwrap().contains_key("a"));
    chrome.end_chat("a", true);
    assert!(!chrome.chat_profiles.lock().unwrap().contains_key("a"));
}
