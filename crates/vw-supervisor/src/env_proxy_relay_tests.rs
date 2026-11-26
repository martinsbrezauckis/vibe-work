//! Current-relay exceptions to the child credential policy. Uses synthetic capabilities only.
//! Called by supervisor unit tests; checks exact ownership, inherited inputs and child inheritance.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

const RELAY: &str = "http://local:current-capability@127.0.0.1:19000";
const PROXIES: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

fn child(
    parent: &[(&str, &str)],
    extra: &[(&str, &str)],
    allowed: &[OsString],
) -> Vec<(OsString, OsString)> {
    let extra: Vec<_> = extra
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).into()))
        .collect();
    build_env_with_proxy_relays(
        &EnvPlan {
            parent: parent
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            extra: &extra,
            vibe_home: Path::new("/relay-test-home"),
            home: None,
            key: None,
            mcp_token: None,
        },
        allowed,
    )
    .unwrap()
}

fn get<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    env.iter()
        .find(|(k, _)| same_name(k, OsStr::new(name)))
        .map(|(_, v)| v.as_os_str())
}

#[test]
fn active_app_relay_reaches_server_and_its_shell_child() {
    let extra = [
        ("HTTP_PROXY", RELAY),
        ("HTTPS_PROXY", RELAY),
        ("http_proxy", RELAY),
        ("https_proxy", RELAY),
        ("NO_PROXY", "inside.test,localhost,127.0.0.1,::1"),
        ("no_proxy", "inside.test,localhost,127.0.0.1,::1"),
    ];
    let env = child(
        &[("HTTPS_PROXY", "http://person:password@corp.test:8080")],
        &extra,
        &[RELAY.into()],
    );
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
        assert_eq!(get(&env, name), Some(OsStr::new(RELAY)), "{name}");
    }
    for name in ["NO_PROXY", "no_proxy"] {
        assert_eq!(
            get(&env, name),
            Some(OsStr::new("inside.test,localhost,127.0.0.1,::1"))
        );
    }
    #[cfg(unix)]
    {
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "test -n \"$HTTP_PROXY\" && test \"$HTTP_PROXY\" = \"$HTTPS_PROXY\"",
            ])
            .env_clear()
            .envs(env)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "shell child did not inherit the relay"
        );
    }
}

#[test]
fn person_password_and_stale_relays_are_stripped_in_every_proxy_slot() {
    for name in PROXIES {
        for rejected in [
            "http://person:password@corp.test:8080",
            "http://person:password@127.0.0.1:19000",
            "http://local:stale-capability@127.0.0.1:19000",
            "http://local:current-capability@127.0.0.1:19001",
            "http://local:current-capability@localhost:19000",
        ] {
            for (parent, extra) in [
                (vec![(name, rejected)], vec![]),
                (vec![], vec![(name, rejected)]),
            ] {
                let env = child(&parent, &extra, &[RELAY.into()]);
                assert_eq!(
                    get(&env, name),
                    None,
                    "{name} retained an unowned credential"
                );
            }
        }
    }
}

#[test]
fn absent_relay_preserves_existing_filtering_and_no_proxy_behavior() {
    for name in PROXIES {
        for allowed in [vec![], vec![OsString::new()]] {
            assert_eq!(get(&child(&[(name, RELAY)], &[], &allowed), name), None);
            assert_eq!(
                get(
                    &child(&[(name, "http://corp.test:8080")], &[], &allowed),
                    name
                ),
                Some(OsStr::new("http://corp.test:8080"))
            );
        }
    }
    assert_eq!(get(&child(&[], &[], &[]), "NO_PROXY"), None);
    assert_eq!(
        get(&child(&[("NO_PROXY", "inside.test")], &[], &[]), "NO_PROXY"),
        Some(OsStr::new("inside.test"))
    );
}

#[test]
fn relay_allowance_is_per_build_and_never_restores_other_credentials() {
    let old = "http://local:second-chat@127.0.0.1:19001";
    let first = child(
        &[],
        &[("HTTPS_PROXY", RELAY), ("GH_TOKEN", RELAY)],
        &[RELAY.into()],
    );
    let second = child(&[], &[("HTTPS_PROXY", RELAY)], &[old.into()]);
    assert_eq!(get(&first, "HTTPS_PROXY"), Some(OsStr::new(RELAY)));
    assert_eq!(get(&first, "GH_TOKEN"), None);
    assert_eq!(get(&second, "HTTPS_PROXY"), None);
    let mut ordinary = vec![("HTTPS_PROXY".into(), RELAY.into())];
    crate::isolate_credentials(&mut ordinary);
    assert_eq!(get(&ordinary, "HTTPS_PROXY"), None);
}

#[test]
fn simultaneous_chat_environments_do_not_share_relay_allowances() {
    let barrier = std::sync::Barrier::new(2);
    let second = "http://local:second-chat@127.0.0.1:19001";
    std::thread::scope(|scope| {
        for (ours, other) in [(RELAY, second), (second, RELAY)] {
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    let env = child(
                        &[],
                        &[("HTTP_PROXY", ours), ("HTTPS_PROXY", other)],
                        &[ours.into()],
                    );
                    assert_eq!(get(&env, "HTTP_PROXY"), Some(OsStr::new(ours)));
                    assert_eq!(get(&env, "HTTPS_PROXY"), None);
                }
            });
        }
    });
}
