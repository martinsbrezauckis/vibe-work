//! Consent label regressions for the native provider card.
//! Depends on the scope formatter; called by the crate's unit-test runner.
//! Keeps wider access explicit and unknown scopes quoted without guessing.

use super::describe_scope;

#[test]
fn wider_scope_labels_do_not_understate_access() {
    let cases = [
        ("Mail.ReadWrite", "Read or change your mail"),
        ("Files.ReadWrite.All", "Read or change your files"),
        ("Calendars.ReadWrite", "Read or change your calendar"),
        (
            "https://www.googleapis.com/auth/contacts",
            "Read or change your contacts",
        ),
        (
            "https://www.googleapis.com/auth/admin.reports.audit.readonly",
            "Unrecognized permission",
        ),
        (
            "https://www.googleapis.com/auth/admin.reports.usage.readonly",
            "Unrecognized permission",
        ),
        ("Mail.Send", "Send mail"),
        ("https://www.googleapis.com/auth/gmail.send", "Send mail"),
    ];
    let failures: Vec<_> = cases
        .iter()
        .filter_map(|(scope, label)| {
            let actual = describe_scope(scope);
            let expected = format!("{label} (\"{scope}\")");
            (actual != expected).then(|| format!("{scope}: expected {expected:?}, got {actual:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn common_scope_labels_keep_the_correct_service_and_access() {
    let cases: &[(&[&str], &str)] = &[
        (
            &[
                "openid",
                "profile",
                "name",
                "read:user",
                "User.Read",
                "https://www.googleapis.com/auth/userinfo.profile",
            ],
            "Read your basic profile",
        ),
        (
            &[
                "email",
                "user:email",
                "https://www.googleapis.com/auth/userinfo.email",
            ],
            "Read your email address",
        ),
        (&["offline_access"], "Keep access when you are away"),
        (
            &[
                "Mail.Read",
                "https://www.googleapis.com/auth/gmail.readonly",
            ],
            "Read your mail",
        ),
        (
            &[
                "Files.Read",
                "Files.Read.All",
                "https://www.googleapis.com/auth/drive.readonly",
            ],
            "Read your files",
        ),
        (
            &[
                "Files.ReadWrite",
                "https://www.googleapis.com/auth/drive",
                "https://www.googleapis.com/auth/drive.file",
            ],
            "Read or change your files",
        ),
        (
            &[
                "Calendars.Read",
                "https://www.googleapis.com/auth/calendar.readonly",
                "https://www.googleapis.com/auth/calendar.events.readonly",
            ],
            "Read your calendar",
        ),
        (
            &[
                "https://www.googleapis.com/auth/calendar",
                "https://www.googleapis.com/auth/calendar.events",
            ],
            "Read or change your calendar",
        ),
        (
            &[
                "Contacts.Read",
                "https://www.googleapis.com/auth/contacts.readonly",
            ],
            "Read your contacts",
        ),
        (&["Contacts.ReadWrite"], "Read or change your contacts"),
        (&["public_repo"], "Read or change your repositories"),
    ];
    for (scopes, label) in cases {
        for scope in *scopes {
            assert_eq!(describe_scope(scope), format!("{label} (\"{scope}\")"));
        }
    }
}

#[test]
fn broader_gmail_scopes_also_name_sending() {
    for (scope, label) in [
        (
            "https://www.googleapis.com/auth/gmail.compose",
            "Read or change drafts and send mail",
        ),
        (
            "https://www.googleapis.com/auth/gmail.modify",
            "Read or change your mail and send mail",
        ),
        (
            "https://mail.google.com/",
            "Read, change, send or permanently delete all your mail",
        ),
    ] {
        assert_eq!(describe_scope(scope), format!("{label} (\"{scope}\")"));
    }
}

#[test]
fn repository_scope_names_organization_resources() {
    assert_eq!(
        describe_scope("repo"),
        "Manage repositories and organization projects, invitations, team memberships and webhooks (\"repo\")",
    );
}

#[test]
fn unknown_scopes_never_inherit_a_familiar_label() {
    for scope in [
        "custom.mail.readwrite",
        "files.read.manage",
        "calendar.read.send",
        "contacts.manage",
        "repo:readwrite",
        "contents:read",
        "offline_admin",
        "user.readwrite",
        "https://example.com/userinfo.profile",
        "https://example.com/userinfo.email",
        "https://example.com/drive.readonly",
        "https://www.googleapis.com/auth/drive.readonly.manage",
        "https://www.googleapis.com/auth/gmail.readonly.send",
        "Mail.ReadWrite.Custom",
        "Mail.Read.Custom",
        "mail.read",
        "MAIL.READ",
        "openid.custom",
        "unknown",
        "",
    ] {
        assert_eq!(
            describe_scope(scope),
            format!("Unrecognized permission (\"{scope}\")")
        );
    }
}

#[test]
fn unknown_scope_keeps_the_raw_value_visibly_escaped() {
    assert_eq!(
        describe_scope("mail.\"read\"\n\u{202e}"),
        "Unrecognized permission (\"mail.\\\"read\\\"\\u000A\\u202E\")",
    );
}
