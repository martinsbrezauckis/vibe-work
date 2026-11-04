//! Shared URL/type/top-page vectors for compiled rules, including saved Strict excerpts and ordering.
//! Run on Linux as well as native Windows/macOS; no test downloads lists.
use super::Matcher;
use crate::shields::{Mode, compile};
use serde_json::json;

const EXCERPT: &str = "[Adblock Plus 2.0]\n||ads.example^$third-party\n@@||ads.example/allowed.js$script\n||metrics.example^$image,domain=news.example\nnews.example##.sponsor\n";

#[test]
fn shared_balanced_vectors() {
    let matcher = Matcher::new(&compile(Mode::Balanced, None).unwrap()).unwrap();
    for (url, kind, page, top, blocked) in [
        (
            "https://doubleclick.net/ad.js",
            "script",
            "https://news.example/",
            false,
            true,
        ),
        (
            "https://a.doubleclick.net/ad.js",
            "script",
            "https://news.example/",
            false,
            true,
        ),
        (
            "https://notdoubleclick.net/ad.js",
            "script",
            "https://news.example/",
            false,
            false,
        ),
        (
            "https://doubleclick.net/",
            "document",
            "https://news.example/",
            true,
            false,
        ),
        (
            "https://doubleclick.net/frame",
            "document",
            "https://news.example/",
            false,
            true,
        ),
        (
            "https://doubleclick.net/ad.js",
            "script",
            "https://doubleclick.net/",
            false,
            false,
        ),
        (
            "http://doubleclick.net/ad.js",
            "script",
            "https://doubleclick.net/",
            false,
            true,
        ),
        (
            "https://doubleclick.net:8443/ad.js",
            "raw",
            "https://doubleclick.net/",
            false,
            true,
        ),
        (
            "https://news.example/story",
            "document",
            "https://news.example/",
            true,
            false,
        ),
    ] {
        let answer = matcher.decide(url, kind, page, top);
        assert_eq!(answer.blocked, blocked, "{url} ({kind}, {page}, top={top})");
    }
    assert_eq!(
        matcher
            .decide(
                "https://news.example/",
                "document",
                "https://news.example/",
                true
            )
            .selectors,
        [".adsbygoogle, [data-ad-slot], iframe[id^='google_ads_iframe_']"]
    );
    assert_eq!(
        Matcher::new("[]").unwrap().decide(
            "https://doubleclick.net/x",
            "script",
            "https://news.example/",
            false
        ),
        super::Decision::default()
    );
}

#[test]
fn shared_strict_excerpt_vectors() {
    let compiled = compile(Mode::Strict, Some(EXCERPT)).unwrap();
    let matcher = Matcher::new(&compiled).unwrap();
    for (url, kind, page, blocked) in [
        (
            "https://ads.example/ad.js",
            "script",
            "https://news.example/",
            true,
        ),
        (
            "https://ads.example/allowed.js",
            "script",
            "https://news.example/",
            false,
        ),
        (
            "https://ads.example/allowed.js",
            "image",
            "https://news.example/",
            true,
        ),
        (
            "https://metrics.example/pixel",
            "image",
            "https://news.example/",
            true,
        ),
        (
            "https://metrics.example/pixel",
            "image",
            "https://other.example/",
            false,
        ),
        (
            "https://metrics.example/pixel",
            "script",
            "https://news.example/",
            false,
        ),
    ] {
        assert_eq!(
            matcher.decide(url, kind, page, false).blocked,
            blocked,
            "{url} {kind} {page}"
        );
    }
    let selectors = matcher
        .decide(
            "https://news.example/",
            "document",
            "https://news.example/",
            true,
        )
        .selectors;
    // The converter puts its first-party-document exception after list cosmetics. Preserve that order.
    assert!(
        !selectors.iter().any(|s| s.contains(".sponsor")),
        "{selectors:?}"
    );
    assert!(
        matcher
            .decide(
                "https://frame.example/",
                "document",
                "https://news.example/",
                false
            )
            .selectors
            .iter()
            .any(|s| s == ".sponsor")
    );
    assert!(
        selectors.iter().any(|s| s.contains("data-ad-slot")),
        "{selectors:?}"
    );
}

#[test]
fn ordered_exceptions_domains_case_and_top_url() {
    let rules = json!([
        {"trigger":{"url-filter":"Ad", "url-filter-is-case-sensitive":true},"action":{"type":"block"}},
        {"trigger":{"url-filter":".*","if-domain":["*news.example"]},"action":{"type":"ignore-previous-rules"}},
        {"trigger":{"url-filter":"Ad","unless-domain":["news.example"],"resource-type":["image"]},"action":{"type":"block"}},
        {"trigger":{"url-filter":".*","if-top-url":["^https://news\\.example/"]},"action":{"type":"css-display-none","selector":".ad"}},
        {"trigger":{"url-filter":"exception"},"action":{"type":"ignore-previous-rules"}},
        {"trigger":{"url-filter":"exception"},"action":{"type":"css-display-none","selector":".own"}}
    ]);
    let matcher = Matcher::new(&rules.to_string()).unwrap();
    assert!(
        !matcher
            .decide(
                "https://cdn.example/ad",
                "script",
                "https://other.example/",
                false
            )
            .blocked
    );
    assert!(
        matcher
            .decide(
                "https://cdn.example/Ad",
                "script",
                "https://other.example/",
                false
            )
            .blocked
    );
    assert!(
        !matcher
            .decide(
                "https://cdn.example/Ad",
                "script",
                "https://a.news.example/",
                false
            )
            .blocked
    );
    assert!(
        matcher
            .decide(
                "https://cdn.example/Ad",
                "image",
                "https://a.news.example/",
                false
            )
            .blocked
    );
    assert!(
        matcher
            .decide(
                "https://cdn.example/Ad",
                "script",
                "https://badnews.example/",
                false
            )
            .blocked
    );
    assert_eq!(
        matcher
            .decide(
                "https://news.example/exception",
                "document",
                "https://news.example/exception",
                true
            )
            .selectors,
        [".own"]
    );
}

#[test]
fn native_fixture_uses_the_same_decision_vectors() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/content-rules.json")).unwrap();
    let matcher = Matcher::new(&data["rules"].to_string()).unwrap();
    for vector in data["vectors"].as_array().unwrap() {
        let url = vector["url"].as_str().unwrap().replace("PORT", "8123");
        let page = vector["page"].as_str().unwrap().replace("PORT", "8123");
        let kind = vector["type"].as_str().unwrap();
        let expected = vector["blocked"].as_bool().unwrap();
        let decision = matcher.decide(&url, kind, &page, false);
        assert_eq!(decision.blocked, expected, "{vector}");
        assert_eq!(
            matcher.blocks(&url, kind, &page, false),
            expected,
            "{vector}"
        );
        assert_eq!(serde_json::json!(decision.selectors), vector["selectors"]);
    }
}
