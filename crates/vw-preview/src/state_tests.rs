//! Refusal bookkeeping of one preview entry: what the window shows and what action receipts read.
use super::Entry;

#[test]
fn a_host_refused_request_is_an_error_the_agent_can_read_without_page_text() {
    let mut first = Entry::new("first");
    first.url = Some(url::Url::parse("https://other.example/private/page?token=hidden").unwrap());
    let second = Entry::new("second");
    first.note_request_refused(
        "10.56.4.13",
        18115,
        "http://example.com may not reach this computer or the local network: the question was not answered",
    );
    assert_eq!(first.console.len(), 1);
    assert_eq!(first.console[0].level, crate::ConsoleLevel::Error);
    assert_eq!(
        first.console[0].text,
        "Browser request refused by Vibe Work: http://example.com may not reach this computer or the local network: the question was not answered"
    );
    assert_eq!(
        first.request_refusals_seen, 1,
        "action receipts keep a lifetime mark"
    );
    assert_eq!(
        first.request_refusals.front().map(|row| row.reason),
        Some("question not answered")
    );
    let row = first.request_refusals.front().unwrap();
    assert_eq!(row.origin.as_deref(), Some("http://example.com"));
    assert_eq!(row.target, "10.56.4.13:18115");
    assert_eq!(row.target_class, "local-network");
    assert!(row.at_ms > 0);
    assert!(!format!("{row:?}").contains("private/page"));
    assert!(!format!("{row:?}").contains("hidden"));
    assert_eq!(first.request_refusals_on_page().len(), 1);
    assert!(second.console.is_empty(), "another chat has no refusal");
    first.reset_console();
    assert!(
        first.console.is_empty(),
        "a new page starts a new diagnostic ring"
    );
    assert_eq!(
        first.request_refusals.len(),
        1,
        "the action receipt survives a page switch"
    );
    assert_eq!(first.request_refusals_seen, 1);
    assert!(
        first.request_refusals_on_page().is_empty(),
        "network.list must not show a prior page's refusal"
    );
    first.note_request_refused(
        "10.56.4.13",
        18115,
        "http://example.com is blocked from reaching apps and devices on this computer or your local network",
    );
    assert_eq!(
        first.request_refusals_on_page()[0].reason,
        "blocked by the user"
    );
    assert_eq!(
        first.request_refusals_on_page()[0].target_class,
        "local-network"
    );
}

#[test]
fn a_mac_own_address_rule_refusal_has_no_guessed_origin_or_page_path() {
    let mut entry = Entry::new("mac-page");
    entry.url = Some(url::Url::parse("https://allowed.example/private?secret=1").unwrap());
    entry.note_request_refused(
        "10.55.5.19",
        18115,
        "web pages cannot reach this Mac's own network address on macOS",
    );
    let row = entry.request_refusals.front().unwrap();
    assert_eq!(row.origin, None);
    assert_eq!(row.target, "10.55.5.19:18115");
    assert_eq!(row.target_class, "own-address");
    assert_eq!(row.reason, "policy");
    assert_eq!(
        row.message,
        "Browser request refused by Vibe Work: web pages cannot reach this Mac's own network address on macOS"
    );
    assert!(!format!("{row:?}").contains("private"));
}

/// A refused third-party page belongs to the page that asked for it: once another page starts loading, the
/// preview no longer shows "Not opened: ..." for it. Receipts still count the refusal (their lifetime mark).
#[test]
fn a_new_page_load_drops_the_previous_pages_refusal_from_the_view() {
    let mut entry = Entry::new("a");
    entry.note_refused("https://third.example/login", "not allowed from this page");
    assert_eq!(
        entry.view(true).refused,
        vec!["https://third.example/login".to_owned()]
    );
    entry.reset_console();
    assert_eq!(entry.view(true).refused, Vec::<String>::new());
    assert_eq!(entry.refusals_seen, 1, "receipts keep their lifetime mark");
    // A refusal of the new page shows again.
    entry.note_refused("https://other.example/", "not allowed from this page");
    assert_eq!(
        entry.view(true).refused,
        vec!["https://other.example/".to_owned()]
    );
}

/// The chats' servers a Local page may reach come from the core's attribution scan once it reports: a preview
/// entry's admitted ports (a free leased port among them) no longer count, Vibe Work's own never.
/// PREDICT (before the fix): every entry's admitted ports and every chat's leased ports counted, so the free
/// leased port 23500 was reachable.
/// A chat server the latest scan found is reached only while it still runs: once its process exited, another
/// program may listen on the port before the next scan.
/// PREDICT (before the check): `chat_ports` returned every port of the last scan until the next one
/// (up to 3 s), so a port whose server had exited stayed admitted; the assertion on 23602 fails.
#[test]
fn a_chat_server_whose_process_exited_is_not_reached() {
    let mut state = super::State::default();
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let check = running.clone();
    let alive: crate::ServerAlive = std::sync::Arc::new(move |port| {
        port != 23602 || check.load(std::sync::atomic::Ordering::SeqCst)
    });
    state.chat_servers = Some((1, vec![23501, 23602], alive));
    assert_eq!(state.chat_ports("a", Some(&[])), vec![23501, 23602]);
    running.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(state.chat_ports("a", Some(&[])), vec![23501]);
}

#[test]
fn the_chats_servers_are_the_attributed_listeners_once_scanned() {
    let mut state = super::State::default();
    state.entry("a").allowed = vec![23500, 23501];
    assert_eq!(
        state.chat_ports("a", Some(&[])),
        vec![23500, 23501],
        "no attribution source yet"
    );
    state.chat_servers = Some((1, vec![23501, 23602, 57497], std::sync::Arc::new(|_| true)));
    assert_eq!(state.chat_ports("a", Some(&[57497])), vec![23501, 23602]);
    state.chat_servers = Some((2, Vec::new(), std::sync::Arc::new(|_| true)));
    assert!(
        state.chat_ports("b", Some(&[])).is_empty(),
        "every server stopped"
    );
    assert_eq!(
        state.chat_ports("a", None),
        vec![23500, 23501],
        "own ports only while Vibe Work's are unknown"
    );
}

/// A request asks the running-server check about its own port only, with the same answer as the full list.
/// PREDICT (before the one-port answer): every request checked every chat server's port (each check reads the
/// socket table): the first request asked about all three ports.
#[test]
fn a_request_checks_only_the_port_it_asks_for() {
    let mut state = super::State::default();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = asked.clone();
    let alive: crate::ServerAlive = std::sync::Arc::new(move |port| {
        seen.lock().unwrap().push(port);
        port != 23602
    });
    state.chat_servers = Some((1, vec![23501, 23602, 57497], alive));
    assert!(state.chat_port_allowed("a", Some(&[]), 23501));
    assert_eq!(*asked.lock().unwrap(), vec![23501]);
    assert!(
        !state.chat_port_allowed("a", Some(&[]), 23602),
        "its server stopped"
    );
    assert!(
        !state.chat_port_allowed("a", Some(&[57497]), 57497),
        "Vibe Work's own"
    );
    assert!(
        !state.chat_port_allowed("a", Some(&[]), 23999),
        "no chat's server"
    );
    assert_eq!(*asked.lock().unwrap(), vec![23501, 23602]);
    state.chat_servers = None;
    state.entry("a").allowed = vec![23500];
    assert!(
        state.chat_port_allowed("a", Some(&[]), 23500),
        "no attribution source yet"
    );
    assert!(!state.chat_port_allowed("a", None, 23501));
}

/// Load waits (`Service::load_changes`) wake on the announcement instead of polling: every loading change and every
/// refused navigation must tick it, or a wait sleeps to its deadline. PREDICT (before the setter): no announcement
/// existed; the waits re-read the state every 60 ms.
#[test]
fn every_loading_change_and_refusal_is_announced() {
    let mut entry = Entry::new("a");
    let mut changes = entry.load_events.subscribe();
    entry.set_loading(true);
    assert!(changes.has_changed().unwrap(), "load start");
    changes.mark_unchanged();
    entry.set_loading(false);
    assert!(changes.has_changed().unwrap(), "load end");
    changes.mark_unchanged();
    entry.note_refused("https://third.example/login", "not allowed from this page");
    assert!(changes.has_changed().unwrap(), "refusal");
    changes.mark_unchanged();
    entry.reset_console();
    assert!(
        !changes.has_changed().unwrap(),
        "no load change, no announcement"
    );
}

/// A page that is being opened is starting, not paused. PREDICT (red before the change): the view reported
/// `Suspended` for every entry without a ready mount, so while the agent's browser opened its first page (the open
/// sets `loading`, then the engine creates the mount) the Browser tab said "Browser paused" with Reload in a status
/// region (NVDA spoke it although nothing was paused) and the agent's preview tool said "paused to save memory".
#[test]
fn a_page_being_opened_is_running_and_only_a_released_engine_is_suspended() {
    use super::{Mount, MountState, PreviewState};
    let mut entry = Entry::new("a");
    entry.url = Some(url::Url::parse("https://example.com/").unwrap());
    // Opened: loading, the engine has not made the mount yet.
    entry.set_loading(true);
    assert_eq!(entry.view(true).state, PreviewState::Live);
    assert_eq!(entry.view(false).state, PreviewState::Hidden);
    // The mount exists and is still being created.
    entry.mount = Some(Mount {
        navigation: crate::navigation::Navigation::default(),
        label: "preview-a".to_owned(),
        state: MountState::Unready,
        page: String::new(),
        class: crate::policy::PageClass::Web,
        names_later: false,
        rules: None,
        rules_pending: true,
        rules_offered: None,
    });
    entry.set_loading(false);
    assert_eq!(entry.view(true).state, PreviewState::Live);
    assert_eq!(entry.view(false).state, PreviewState::Hidden);
    // Released to respect the live cap: no mount, nothing loading.
    entry.mount = None;
    assert_eq!(entry.view(true).state, PreviewState::Suspended);
    assert_eq!(entry.view(false).state, PreviewState::Suspended);
}

/// PREDICT (fails before the change): a refusal of a request that did not say which open page sent it had no reason
/// of its own (it read as "policy") and nothing said that the sender was unknown.
#[test]
fn an_unattributed_refusal_says_the_sender_is_unknown() {
    let mut entry = Entry::new("page");
    let reason = crate::local_access::UNANSWERED_TEXT;
    entry.note_request_refused_by("10.55.5.17", 18115, reason, false);
    let row = entry.request_refusals_on_page().pop().unwrap();
    assert_eq!(row.reason, "sender unknown");
    assert_eq!(row.target_class, "local-network");
    assert_eq!(row.target, "10.55.5.17:18115");
    assert_eq!(row.origin, None);
    let json = serde_json::json!(row);
    assert_eq!(json["senderKnown"], false, "{json}");
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .starts_with("Browser request refused by Vibe Work: several web pages are open"),
        "{json}"
    );
    // A refusal whose page is known keeps the earlier shape (no senderKnown field).
    entry.note_request_refused("10.55.5.17", 18115, reason);
    assert!(
        serde_json::json!(entry.request_refusals_on_page().pop().unwrap())
            .get("senderKnown")
            .is_none()
    );
}

/// Three pages of three chats on one store: `a.example` and `b.example` undecided, `c.example` allowed.
fn three_chats_on_one_store() -> (crate::state::State, Vec<String>) {
    let mut state = crate::state::State::default();
    let mut labels = Vec::new();
    for chat in ["a", "b", "c"] {
        let page = format!("web-page-{chat}");
        let mut entry = Entry::new(&page);
        entry.url = Some(url::Url::parse(&format!("http://{chat}.example/")).unwrap());
        state.entries.insert(page.clone(), entry);
        state.page_owners.insert(page.clone(), chat.to_owned());
        let label = format!("mount-{chat}");
        state.by_label.insert(label.clone(), page);
        labels.push(label);
    }
    (state, labels)
}

/// PREDICT (fails before the change): a "sender unknown" refusal reached only the chats with an undecided page on the
/// store. macOS `WebKit` sends plain http through a tunnel that names no sender, so with two tabs open the page that
/// really sent it was often one whose site is allowed: that agent saw only "Load failed" and no refusal row. Every
/// page of the store could have sent it, so every page now gets the row and its console line.
#[test]
fn a_sender_unknown_refusal_reaches_every_page_that_could_have_sent_it() {
    let (mut state, labels) = three_chats_on_one_store();
    // c.example is allowed (Settings): its request is refused too while another open site is undecided.
    let reason = crate::local_access::UNANSWERED_TEXT;
    state.note_unattributed_refusal(&labels, "10.55.5.17", 80, reason);
    for chat in ["a", "b", "c"] {
        let entry = &state.entries[&format!("web-page-{chat}")];
        let rows = entry.request_refusals_on_page();
        assert_eq!(rows.len(), 1, "{chat}: {rows:?}");
        assert_eq!(rows[0].reason, "sender unknown");
        assert_eq!(rows[0].target, "10.55.5.17:80");
        assert_eq!(rows[0].sender_known, Some(false));
        assert!(
            entry.console[0]
                .text
                .starts_with("Browser request refused by Vibe Work: several web pages are open"),
            "{chat}: the approved console line"
        );
    }
}

/// A refusal whose reason names one open site stays with the chat of that site's page; other chats get nothing.
#[test]
fn an_unattributed_refusal_naming_a_site_reaches_only_that_sites_chat() {
    let (mut state, labels) = three_chats_on_one_store();
    let reason = crate::local_access::blocked_text("http://b.example");
    state.note_unattributed_refusal(&labels, "10.55.5.17", 80, &reason);
    for (chat, rows) in [("a", 0), ("b", 1), ("c", 0)] {
        let entry = &state.entries[&format!("web-page-{chat}")];
        assert_eq!(entry.request_refusals_on_page().len(), rows, "{chat}");
    }
}
