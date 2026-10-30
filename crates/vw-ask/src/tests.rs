//! Pure permission outcome and policy contract tests, run without a presenter or clock.
use super::*;
#[test]
fn every_outcome_has_one_plain_agent_sentence() {
    let refused = Outcome::Refused {
        by: AnsweredBy::Person,
        note: Some("  a plain *note*  ".into()),
    };
    // A No answers the declined call with one fixed sentence, whatever was asked and whatever note came with it.
    let declined = "Not run: the user declined this. Do not do it another way. Tell the user why it is needed and wait for their reply before you go on with it.";
    assert_eq!(agent_text(&refused, "this command"), declined);
    assert_eq!(
        agent_text(
            &Outcome::Refused {
                by: AnsweredBy::DebugApi,
                note: None
            },
            "anything"
        ),
        declined
    );
    assert_eq!(
        agent_text(
            &Outcome::Cancelled {
                reason: "the turn was stopped".into()
            },
            "anything"
        ),
        "Not done: the turn was stopped before the user answered."
    );
    assert_eq!(
        agent_text(&Outcome::Never, "saved logins"),
        "Not done: the user set saved logins to Never in Vibe Work Settings."
    );
    assert_eq!(
        agent_text(
            &Outcome::Allowed {
                by: AnsweredBy::Person,
                scope: Scope::Once
            },
            "command"
        ),
        ""
    );
    assert_eq!(plain_note("\u{0007}x\t"), "\\u{7}x");
    assert_eq!(plain_note(&"x".repeat(2100)).len(), 2000);
}
#[test]
fn vault_policies_are_native_and_person_only() {
    for kind in [
        "vibe-work/secretRun",
        "vibe-work/fillLogin",
        "vibe-work/mcpServerKey",
        "vibe-work/captureSecret",
        "vibe-work/replaceModelKey",
    ] {
        let row = policy(kind).unwrap();
        assert!(row.person_only);
        assert_eq!(row.surface, Surface::Native);
    }
    assert_eq!(policy("approval").unwrap().surface, Surface::Card);
}

/// The 2,000-character limit holds for the text the agent reads, after control characters are
/// written visibly (each becomes up to 10 characters).
#[test]
fn a_plain_note_stays_within_its_limit_after_escaping() {
    assert!(plain_note(&"\u{1b}".repeat(2000)).chars().count() <= 2000);
    assert!(plain_note(&"\u{1b}x".repeat(1500)).chars().count() <= 2000);
}

/// A grant covers exactly its session, tool and permission set (order-free); a literal scope never stands in
/// for the same text as a glob; an every-call grant covers only calls without permissions.
#[test]
fn a_session_grant_covers_only_an_equal_permission_set() {
    let npm = GrantPermission::from_wire("command_pattern", "npm install x", "npm install *");
    let out = GrantPermission::from_wire("outside_directory", "/tmp/a", "vibe-path:exact:/tmp/a");
    assert!(!npm.exact);
    assert!(
        !out.exact,
        "an encoded path grant is not the invocation's text"
    );
    assert!(GrantPermission::from_wire("command_pattern", "git  log  *", "git log *").exact);
    // b1r: Python's str.split() also splits on \x1c..\x1f, so Vibe's literal `git log *` from this text is exact.
    assert!(GrantPermission::from_wire("command_pattern", "git\u{1f}log *", "git log *").exact);
    assert!(GrantPermission::from_wire("command_pattern", "a\u{1c}\u{1d}b", "a b").exact);
    let grant = SessionGrant {
        id: "grant-1".into(),
        session: "s".into(),
        tool: "file_system.bash".into(),
        permissions: vec![npm.clone(), out.clone()],
        labels: vec![],
        every_call: false,
        created_at: 0,
        by: AnsweredBy::Person,
        covers: vec![],
        uses: 0,
        last_used_at: None,
    };
    assert!(grant.covers("s", "file_system.bash", &[out.clone(), npm.clone()]));
    assert!(
        !grant.covers("s", "file_system.bash", std::slice::from_ref(&npm)),
        "a subset is asked"
    );
    assert!(!grant.covers("other", "file_system.bash", &[npm.clone(), out.clone()]));
    assert!(!grant.covers("s", "process.start", &[npm.clone(), out.clone()]));
    let literal = GrantPermission::from_wire("command_pattern", "git log *", "git log *");
    let glob = GrantPermission::from_wire("command_pattern", "git log --ext-diff", "git log *");
    assert_ne!(literal, glob);
    let tool = SessionGrant {
        permissions: vec![],
        every_call: true,
        tool: "mcp_notes.list".into(),
        ..grant
    };
    assert!(tool.covers("s", "mcp_notes.list", &[]));
    assert!(
        !tool.covers("s", "mcp_notes.list", &[npm]),
        "a call with permissions is another question"
    );
}

/// "Always" only for rows naming a separate Settings control: "Close tabs you opened" on its Chrome card.
#[test]
fn no_policy_row_offers_always_without_a_settings_control() {
    for kind in policies() {
        let row = policy(kind).unwrap();
        assert_eq!(
            row.always_setting,
            always_settings().find(|(k, _)| *k == kind).map(|(_, s)| s),
            "{kind}"
        );
    }
    let named: Vec<_> = always_settings().collect();
    assert_eq!(
        named,
        vec![(chrome::CLOSE_TABS_KIND, "browser-close-user-tabs")]
    );
    for group in chrome::Group::ALL {
        let row = policy(group.ask_kind()).unwrap();
        assert_eq!(row.surface, Surface::Card);
        assert_eq!(
            row.always_setting, None,
            "no card raises the level: {group:?}"
        );
    }
    assert_eq!(policy(chrome::OTHER_KIND).unwrap().always_setting, None);
    assert!(!policy(chrome::LOOSEN_KIND).unwrap().person_only);
}

#[test]
fn owner_debug_api_can_answer_non_vault_restorations_and_permission_changes() {
    for kind in [
        chrome::LOOSEN_KIND,
        chrome::RESTORE_KIND,
        "newChat/restoreFolder",
    ] {
        let row = policy(kind).unwrap();
        assert_eq!(row.surface, Surface::Native);
        assert!(
            !row.person_only,
            "{kind}: only vault actions require a person"
        );
    }
}
#[test]
fn notes_show_hidden_format_characters_as_escapes() {
    // Direction overrides, zero-width and line/paragraph separators would change how the note reads to the agent
    // and in the records; they are written out like control characters.
    assert_eq!(
        plain_note("ok\u{202E}gnp.exe\u{200B} done\u{2028}x\u{FEFF}"),
        "ok\\u{202e}gnp.exe\\u{200b} done\\u{2028}x\\u{feff}"
    );
    assert_eq!(plain_note("line one\nline two"), "line one\nline two");
}
