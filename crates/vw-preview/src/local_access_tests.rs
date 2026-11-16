//! The Local Network Access verdict per store and the remembered answers.
//!
//! PREDICT (before the change): no verdict existed; every live browser mount reached every address class, so a
//! public page's request to `127.0.0.1`, a LAN address or `169.254.169.254` was sent without asking, an App preview
//! subresource reached any local port, and nothing was remembered per site. Each case below then had no answer at all
//! (the module did not exist): these tests are the specification the guard context follows.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

fn request(class: IpClass, port: u16, initiator: Option<&'static str>) -> LocalRequest<'static> {
    LocalRequest {
        class,
        port,
        host: "127.0.0.1",
        own_address: false,
        initiator,
        nested: false,
    }
}

fn public(site: &str) -> Member {
    Member::Public {
        site: Some(site.to_owned()),
        session: "chat".to_owned(),
        navigating: None,
        loaded: true,
    }
}

fn none(_: &str) -> Option<SiteAnswer> {
    None
}

#[test]
fn a_public_page_is_asked_about_its_own_site_for_every_local_class() {
    for class in [
        IpClass::Loopback,
        IpClass::Private,
        IpClass::LinkLocal,
        IpClass::SharedCgnat,
    ] {
        assert_eq!(
            verdict(
                &[public("https://a.example")],
                &request(class, 8080, None),
                none
            ),
            LocalVerdict::Ask {
                site: "https://a.example".to_owned(),
                session: "chat".to_owned()
            },
            "{class:?}"
        );
    }
}

#[test]
fn allow_and_block_are_per_site() {
    let answers = |site: &str| match site {
        "https://a.example" => Some(SiteAnswer::Allow),
        "https://b.example" => Some(SiteAnswer::Block),
        _ => None,
    };
    let local = request(IpClass::Loopback, 9000, None);
    assert_eq!(
        verdict(&[public("https://a.example")], &local, answers),
        LocalVerdict::Allow
    );
    assert_eq!(
        verdict(&[public("https://b.example")], &local, answers),
        LocalVerdict::Refuse(blocked_text("https://b.example"))
    );
    // Another origin of the allowed host is another site: asked.
    assert!(matches!(
        verdict(&[public("http://a.example")], &local, answers),
        LocalVerdict::Ask { .. }
    ));
}

#[test]
fn pages_opened_on_this_computer_or_the_lan_are_not_asked() {
    let local = Member::Local {
        site: Some("http://127.0.0.1:3000".to_owned()),
    };
    for class in [IpClass::Loopback, IpClass::Private, IpClass::LinkLocal] {
        assert_eq!(
            verdict(
                std::slice::from_ref(&local),
                &request(class, 4000, None),
                none
            ),
            LocalVerdict::Allow
        );
    }
}

#[test]
fn the_app_preview_reaches_only_its_own_ports_on_this_computer() {
    let app = Member::App {
        site: Some("http://127.0.0.1:3000".to_owned()),
        allowed: vec![3000, 3001],
    };
    let own = std::slice::from_ref(&app);
    assert_eq!(
        verdict(own, &request(IpClass::Loopback, 3001, None), none),
        LocalVerdict::Allow
    );
    assert!(matches!(
        verdict(own, &request(IpClass::Loopback, 5432, None), none),
        LocalVerdict::Refuse(reason) if reason.contains("5432")
    ));
    for class in [IpClass::Private, IpClass::LinkLocal] {
        assert!(matches!(
            verdict(own, &request(class, 3000, None), none),
            LocalVerdict::Refuse(_)
        ));
    }
    // This computer's own LAN address is this computer for the question (class Loopback), but the App preview keeps
    // to the loopback spellings its servers were opened on.
    for host in ["198.51.100.77", "dev.example"] {
        let own_spelling = LocalRequest {
            host,
            own_address: true,
            ..request(IpClass::Loopback, 3000, None)
        };
        assert!(
            matches!(verdict(own, &own_spelling, none), LocalVerdict::Refuse(_)),
            "{host}"
        );
    }
    // Attributed to the App preview on a store it shares with an opened local page: still its own ports only.
    let shared = [
        app.clone(),
        Member::Local {
            site: Some("http://127.0.0.1:8080".to_owned()),
        },
    ];
    assert!(matches!(
        verdict(
            &shared,
            &request(IpClass::Loopback, 5432, Some("http://127.0.0.1:3000")),
            none
        ),
        LocalVerdict::Refuse(_)
    ));
    // Unattributed on that store: the App preview may have sent it, so its limits hold (it never borrows the
    // opened local page's rights): another port and link-local metadata are refused, the chats' ports pass.
    // PREDICT (before the fix): Allow for both refused requests below.
    assert!(matches!(
        verdict(&shared, &request(IpClass::Loopback, 5432, None), none),
        LocalVerdict::Refuse(_)
    ));
    assert!(matches!(
        verdict(&shared, &request(IpClass::LinkLocal, 80, None), none),
        LocalVerdict::Refuse(_)
    ));
    assert_eq!(
        verdict(&shared, &request(IpClass::Loopback, 3000, None), none),
        LocalVerdict::Allow
    );
}

#[test]
fn a_shared_web_store_asks_only_when_the_requester_is_clear() {
    let store = [public("https://a.example"), public("https://b.example")];
    // Attributed by its origin: that site is asked.
    assert_eq!(
        verdict(
            &store,
            &request(IpClass::Loopback, 80, Some("https://b.example")),
            none
        ),
        LocalVerdict::Ask {
            site: "https://b.example".to_owned(),
            session: "chat".to_owned()
        }
    );
    // Unattributed with two undecided sites: refused, no one is asked.
    assert!(matches!(
        verdict(&store, &request(IpClass::Loopback, 80, None), none),
        LocalVerdict::Refuse(_)
    ));
    // Unattributed and one site undecided while another is allowed: refused (it may be either), not asked.
    let one_allowed = |site: &str| (site == "https://a.example").then_some(SiteAnswer::Allow);
    assert!(matches!(
        verdict(&store, &request(IpClass::Loopback, 80, None), one_allowed),
        LocalVerdict::Refuse(_)
    ));
    // Every page allowed: sent.
    let all = |_: &str| Some(SiteAnswer::Allow);
    assert_eq!(
        verdict(&store, &request(IpClass::Loopback, 80, None), all),
        LocalVerdict::Allow
    );
    // An initiator that is no member's page is a frame of another site: refused, nobody is asked.
    assert!(matches!(
        verdict(
            &store,
            &request(IpClass::Loopback, 80, Some("https://frame.example")),
            none
        ),
        LocalVerdict::Refuse(_)
    ));
}

#[test]
fn a_page_without_a_site_or_a_closed_store_is_refused() {
    let blank = Member::Public {
        site: None,
        session: "chat".to_owned(),
        navigating: None,
        loaded: true,
    };
    assert!(matches!(
        verdict(&[blank], &request(IpClass::Loopback, 80, None), none),
        LocalVerdict::Refuse(_)
    ));
    assert!(matches!(
        verdict(&[], &request(IpClass::Loopback, 80, None), none),
        LocalVerdict::Refuse(_)
    ));
}

#[test]
fn answers_are_saved_per_site_and_can_be_changed() {
    let dir = std::env::temp_dir().join(format!(
        "vw-local-access-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let root = Root(dir);
    let mut answers = SiteAnswers::load(root.path());
    answers
        .set("https://a.example", Some(SiteAnswer::Allow))
        .unwrap();
    answers
        .set("https://b.example", Some(SiteAnswer::Block))
        .unwrap();
    let again = SiteAnswers::load(root.path());
    assert_eq!(again.get("https://a.example"), Some(SiteAnswer::Allow));
    assert_eq!(again.get("https://b.example"), Some(SiteAnswer::Block));
    let mut again = again;
    again
        .set("https://b.example", Some(SiteAnswer::Allow))
        .unwrap();
    again.set("https://a.example", None).unwrap();
    let last = SiteAnswers::load(root.path());
    assert_eq!(last.get("https://a.example"), None);
    assert_eq!(last.get("https://b.example"), Some(SiteAnswer::Allow));
    // Only exact sites are kept from the file.
    std::fs::write(
        root.path().join(FILE),
        r#"{"sites":{"https://ok.example":"allow","https://x.example/path":"allow","ftp://f.example":"block"}}"#,
    )
    .unwrap();
    assert_eq!(
        SiteAnswers::load(root.path()).list(),
        vec![("https://ok.example".to_owned(), SiteAnswer::Allow)]
    );
    assert_eq!(
        site_of_text("https://Example.com:443/a?b"),
        Some("https://example.com".to_owned())
    );
    assert_eq!(site_of_text("file:///etc/passwd"), None);
}

/// A test folder removed when the test ends.
struct Root(std::path::PathBuf);
impl Root {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_top_level_navigation_to_this_computer_is_not_asked() {
    // PREDICT: without the navigation exemption the verdict asks (single page) or refuses (several pages) for the
    // page's own navigation, e.g. an OAuth redirect back to a local app.
    let navigating = Member::Public {
        site: Some("https://login.example".to_owned()),
        session: "chat".to_owned(),
        navigating: Some(("127.0.0.1".to_owned(), 3000)),
        loaded: true,
    };
    let store = [navigating, public("https://b.example")];
    assert_eq!(
        verdict(&store, &request(IpClass::Loopback, 3000, None), none),
        LocalVerdict::Allow
    );
    // Only that host and port: another port is still a request of the page.
    assert!(matches!(
        verdict(&store, &request(IpClass::Loopback, 3001, None), none),
        LocalVerdict::Refuse(_)
    ));
    // Blocked or not, a navigation away is not a request of the page.
    let blocked = |_: &str| Some(SiteAnswer::Block);
    assert_eq!(
        verdict(&store, &request(IpClass::Loopback, 3000, None), blocked),
        LocalVerdict::Allow
    );
    let url = Url::parse("http://[::1]:8080/cb").unwrap();
    let (host, port, _) = navigation_target(&url).unwrap();
    assert_eq!((host.as_str(), port), ("::1", 8080));
}

#[test]
fn a_frames_navigation_to_this_computer_is_asked_like_a_subresource() {
    // PREDICT: WebKitGTK reports a frame's navigation through the page's navigation callback, so an iframe of
    // a public page to this computer takes the top-level exemption: allowed unasked, even for a blocked site.
    let navigating = Member::Public {
        site: Some("https://public.example".to_owned()),
        session: "chat".to_owned(),
        navigating: Some(("127.0.0.1".to_owned(), 3000)),
        loaded: true,
    };
    let frame = LocalRequest {
        nested: true,
        ..request(IpClass::Loopback, 3000, Some("https://public.example"))
    };
    let store = std::slice::from_ref(&navigating);
    assert!(matches!(
        verdict(store, &frame, none),
        LocalVerdict::Ask { ref site, .. } if site == "https://public.example"
    ));
    let blocked = |_: &str| Some(SiteAnswer::Block);
    assert!(matches!(
        verdict(store, &frame, blocked),
        LocalVerdict::Refuse(_)
    ));
    let allowed = |_: &str| Some(SiteAnswer::Allow);
    assert_eq!(verdict(store, &frame, allowed), LocalVerdict::Allow);
    // The page's own top-level navigation keeps the exemption.
    assert_eq!(
        verdict(
            store,
            &request(IpClass::Loopback, 3000, Some("https://public.example")),
            blocked
        ),
        LocalVerdict::Allow
    );
}

#[test]
fn the_navigation_exemption_is_literal_own_and_single() {
    // PREDICT: a host:port match alone would let a name that rebinds to this computer, or another page's request,
    // ride a navigation.
    let named = Member::Public {
        site: Some("https://a.example".to_owned()),
        session: "chat".to_owned(),
        navigating: Some(("rebind.example".to_owned(), 80)),
        loaded: true,
    };
    let rebinding = LocalRequest {
        class: IpClass::Loopback,
        port: 80,
        host: "rebind.example",
        own_address: false,
        initiator: None,
        nested: false,
    };
    assert!(matches!(
        verdict(std::slice::from_ref(&named), &rebinding, none),
        LocalVerdict::Ask { .. }
    ));
    // A request attributed to another page is not that page's navigation.
    let leaving = Member::Public {
        site: Some("https://a.example".to_owned()),
        session: "chat".to_owned(),
        navigating: Some(("127.0.0.1".to_owned(), 3000)),
        loaded: true,
    };
    let store = [leaving, public("https://b.example")];
    assert_eq!(
        verdict(
            &store,
            &request(IpClass::Loopback, 3000, Some("https://b.example")),
            none
        ),
        LocalVerdict::Ask {
            site: "https://b.example".to_owned(),
            session: "chat".to_owned()
        }
    );
    // The navigating member is reported, so the caller uses the exemption up.
    assert_eq!(
        verdict_with(&store, &request(IpClass::Loopback, 3000, None), none).1,
        Some(0)
    );
}

#[test]
fn a_frame_of_another_site_never_borrows_the_page_answer() {
    // PREDICT: before, an initiator that matched no page counted as a request of every page, so a third-party frame
    // inside an allowed site's page was allowed through that site's answer.
    let page = [public("https://allowed.example")];
    let frame = request(IpClass::Loopback, 80, Some("https://ads.example"));
    let only_page = |site: &str| (site == "https://allowed.example").then_some(SiteAnswer::Allow);
    assert!(matches!(
        verdict(&page, &frame, only_page),
        LocalVerdict::Refuse(_)
    ));
    // Its own Allow alone is not enough: the page must pass too.
    let frame_allowed = |site: &str| (site == "https://ads.example").then_some(SiteAnswer::Allow);
    assert!(matches!(
        verdict(&page, &frame, frame_allowed),
        LocalVerdict::Refuse(_)
    ));
    let both = |_: &str| Some(SiteAnswer::Allow);
    assert_eq!(verdict(&page, &frame, both), LocalVerdict::Allow);
    let page_blocked = |site: &str| match site {
        "https://allowed.example" => Some(SiteAnswer::Block),
        _ => Some(SiteAnswer::Allow),
    };
    assert_eq!(
        verdict(&page, &frame, page_blocked),
        LocalVerdict::Refuse(blocked_text("https://allowed.example"))
    );
    let frame_blocked = |site: &str| match site {
        "https://ads.example" => Some(SiteAnswer::Block),
        _ => Some(SiteAnswer::Allow),
    };
    assert_eq!(
        verdict(&page, &frame, frame_blocked),
        LocalVerdict::Refuse(blocked_text("https://ads.example"))
    );
    // The page's own request still uses its answer.
    assert_eq!(
        verdict(
            &page,
            &request(IpClass::Loopback, 80, Some("https://allowed.example")),
            only_page
        ),
        LocalVerdict::Allow
    );
    // A page opened on this computer keeps its frames (as before).
    let local = [Member::Local {
        site: Some("http://127.0.0.1:3000".to_owned()),
    }];
    assert_eq!(
        verdict(
            &local,
            &request(IpClass::Loopback, 5173, Some("http://127.0.0.1:5173")),
            none
        ),
        LocalVerdict::Allow
    );
}

/// A local page reached from a web page by a top-level link stays in the web page's mount (`Public`), with its own
/// local origin as its site.
///
/// PREDICT (red before the change): its own same-origin requests (`/favicon.ico` of `http://localhost:8765`) were
/// treated like a web page reaching this computer, so Vibe Work asked about `http://localhost:8765` and the person's
/// answer landed in Settings. The first case fails with `Ask` instead of `Allow`. After: a request to the page's own
/// origin passes when that origin is an address literal or `localhost` (the page itself is local); every other local
/// target still asks, and a name never counts as the page's own origin (a DNS answer can point it at this computer).
#[test]
fn a_local_page_reaches_its_own_origin_without_asking() {
    let page = public("http://localhost:8765");
    let own = |host: &'static str, port: u16| LocalRequest {
        class: IpClass::Loopback,
        port,
        host,
        own_address: false,
        initiator: Some("http://localhost:8765"),
        nested: false,
    };
    assert_eq!(
        verdict(std::slice::from_ref(&page), &own("localhost", 8765), none),
        LocalVerdict::Allow
    );
    let ask = LocalVerdict::Ask {
        site: "http://localhost:8765".to_owned(),
        session: "chat".to_owned(),
    };
    assert_eq!(
        verdict(std::slice::from_ref(&page), &own("localhost", 9000), none),
        ask,
        "another port of this computer still asks"
    );
    assert_eq!(
        verdict(std::slice::from_ref(&page), &own("127.0.0.1", 8765), none),
        ask,
        "another host name for the same address is another origin"
    );
    let v6 = public("http://[::1]:8765");
    let to_v6 = LocalRequest {
        host: "::1",
        initiator: Some("http://[::1]:8765"),
        ..own("::1", 8765)
    };
    assert_eq!(verdict(&[v6], &to_v6, none), LocalVerdict::Allow);
    let lan = public("http://10.20.30.40");
    let to_lan = LocalRequest {
        class: IpClass::Private,
        host: "10.20.30.40",
        initiator: Some("http://10.20.30.40"),
        ..own("10.20.30.40", 80)
    };
    assert_eq!(verdict(&[lan], &to_lan, none), LocalVerdict::Allow);
    // The browser's own favicon request names no initiator: it passes once the local page finished loading.
    let unnamed = LocalRequest {
        initiator: None,
        ..own("localhost", 8765)
    };
    assert_eq!(
        verdict(std::slice::from_ref(&page), &unnamed, none),
        LocalVerdict::Allow
    );
    let loading = Member::Public {
        site: Some("http://localhost:8765".to_owned()),
        session: "chat".to_owned(),
        navigating: None,
        loaded: false,
    };
    assert_eq!(
        verdict(&[loading], &unnamed, none),
        ask,
        "while it loads, the web page it came from may still send unnamed requests"
    );
}

#[test]
fn a_name_never_counts_as_the_pages_own_local_origin() {
    // A web site whose name now resolves to this computer (DNS rebinding) is still a web page reaching it.
    let rebound = LocalRequest {
        class: IpClass::Loopback,
        port: 443,
        host: "a.example",
        own_address: false,
        initiator: Some("https://a.example"),
        nested: false,
    };
    assert_eq!(
        verdict(&[public("https://a.example")], &rebound, none),
        LocalVerdict::Ask {
            site: "https://a.example".to_owned(),
            session: "chat".to_owned()
        }
    );
    // A blocked local origin stays blocked for other targets; its own origin is the page itself.
    let blocked = |_: &str| Some(SiteAnswer::Block);
    let other = LocalRequest {
        class: IpClass::Loopback,
        port: 9000,
        host: "localhost",
        own_address: false,
        initiator: Some("http://localhost:8765"),
        nested: false,
    };
    assert_eq!(
        verdict(&[public("http://localhost:8765")], &other, blocked),
        LocalVerdict::Refuse(blocked_text("http://localhost:8765"))
    );
}
