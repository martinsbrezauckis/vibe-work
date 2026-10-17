//! Broker reads and output tests against fake gh, exercising the local endpoint with no real login.
//! The fake proves argv/cwd fidelity, child-only authentication, byte preservation and redaction.

mod support;

use support::{Harness, TOKEN};

#[test]
fn listed_reads_run_with_token_and_original_argv_cwd() {
    let harness = Harness::new();
    let cases: &[&[&str]] = &[
        &["pr", "view", "17"],
        &["pr", "list", "--state", "all", "--limit", "5"],
        &["pr", "checks", "17", "--required", "--json", "name,state"],
        &["pr", "diff", "17", "--patch"],
        &["run", "view", "123", "--log-failed"],
        &["run", "list", "--branch", "main", "--limit", "5"],
        &["issue", "view", "12", "--comments"],
        &["issue", "list", "--assignee", "@me", "--label", "bug"],
        &["repo", "view", "owner/repo", "--json", "name,description"],
        &["api", "repos/{owner}/{repo}/pulls?state=open&per_page=5"],
        &["api", "user", "-X", "GET"],
        &["api", "-XGET", "/user", "--include"],
        &["api", "user", "--method=GET", "--paginate", "--slurp"],
        &[
            "api",
            "https://api.github.com/user",
            "-H",
            "Accept: application/vnd.github+json",
        ],
        &["api", "user", "-H", "X-GitHub-Api-Version: 2022-11-28"],
        &["pr", "view", "--repo", "owner/repo", "--json", "title"],
        &["-Rowner/repo", "pr", "list", "--search", "fix auth"],
        &["issue", "view", "https://github.com/owner/repo/issues/12"],
        &[
            "pr",
            "view",
            "17",
            "-R",
            "github.com/owner/repo",
            "--template",
            "{{.title}}",
        ],
        &["pr", "view", "17", "-R", "https://github.com/owner/repo"],
        &["pr", "checks", "--interval", "5", "--watch", "--fail-fast"],
        &[
            "run",
            "view",
            "123",
            "--exit-status",
            "--job",
            "45",
            "--verbose",
        ],
    ];
    for argv in cases {
        let reply = harness.call(argv);
        let stdout = String::from_utf8_lossy(&reply.stdout);
        assert_eq!(
            reply.exit_code,
            0,
            "{argv:?}: {}",
            String::from_utf8_lossy(&reply.stderr)
        );
        assert!(stdout.contains("gh_token_set=true"), "{stdout}");
        assert!(stdout.contains(&format!("argv={argv:?}")), "{stdout}");
        assert!(
            stdout.contains(&format!("cwd={}", harness.cwd.path().display())),
            "{stdout}"
        );
        assert!(stdout.contains("config_empty=true"), "{stdout}");
        assert!(stdout.contains("unexpected_env=false"), "{stdout}");
    }
}

#[test]
fn redacts_every_token_occurrence_on_both_streams_and_preserves_binary() {
    let reply = Harness::new().call(&["api", "user"]);
    assert!(
        !reply
            .stdout
            .windows(TOKEN.len())
            .any(|part| part == TOKEN.as_bytes())
    );
    assert!(
        !reply
            .stderr
            .windows(TOKEN.len())
            .any(|part| part == TOKEN.as_bytes())
    );
    assert!(String::from_utf8_lossy(&reply.stdout).contains("token=[REDACTED]/[REDACTED]"));
    assert_eq!(reply.stderr, b"stderr-token=[REDACTED]\n");
    assert!(reply.stdout.ends_with(b"binary=\xff\x00\n"));
}

#[test]
fn preserves_nonzero_gh_exit_code() {
    let harness = Harness::new();
    std::fs::write(harness.cwd.path().join("exit-code"), "8").unwrap();
    assert_eq!(harness.call(&["pr", "checks", "17"]).exit_code, 8);
}

#[test]
fn refuses_bad_setup_and_redacts_launch_error_paths() {
    assert!(vw_gh_broker::Broker::bind(String::new(), support::fake_gh()).is_err());
    assert!(vw_gh_broker::Broker::bind(TOKEN.to_owned(), std::path::Path::new("gh")).is_err());
    let harness = Harness::new();
    let mut request = harness.request(&["pr", "view"]);
    request.cwd = harness.cwd.path().join(TOKEN);
    let reply = std::thread::scope(|scope| {
        let server = scope.spawn(|| harness.broker.serve_one().unwrap());
        let reply = vw_gh_broker::call(harness.broker.endpoint(), &request).unwrap();
        server.join().unwrap();
        reply
    });
    assert_eq!(reply.exit_code, 1);
    assert!(!String::from_utf8_lossy(&reply.stderr).contains(TOKEN));
    assert!(!harness.cwd.path().join("runs").exists());
}
