//! Refusal bookkeeping of one preview entry: what the window shows and what action receipts read.
use super::Entry;

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
