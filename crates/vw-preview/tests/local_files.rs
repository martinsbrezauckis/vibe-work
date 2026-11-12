//! Local document admission stays bound to the file explicitly selected by the core, never to the file scheme.
use url::Url;
use vw_preview::UrlPolicy;

#[test]
fn ordinary_navigation_cannot_enable_all_local_files() {
    let policy = UrlPolicy::default();
    assert!(policy.shape("file:///tmp/document.html").is_err());
    // A real local path for this platform: `file:///tmp/...` has no drive letter, so Windows rightly calls it
    // not a local file.
    let file = |name: &str| Url::from_file_path(std::env::temp_dir().join(name)).unwrap();
    let mut heading = file("document.html");
    heading.set_fragment(Some("heading"));
    assert!(vw_preview::same_local_document(
        &file("document.html"),
        &heading
    ));
    assert!(!vw_preview::same_local_document(
        &file("document.html"),
        &file("other.html")
    ));
    assert!(!vw_preview::same_local_document(
        &Url::parse("file://server/share/document.html").unwrap(),
        &Url::parse("file://server/share/document.html").unwrap()
    ));
}
