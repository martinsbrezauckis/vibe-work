//! Write and policy falsifiers sent directly to the broker, bypassing the shim and any hook.
//! Fake gh's invocation file must remain absent for every refusal, including alternate flag spellings.

mod support;

use support::Harness;

#[test]
fn refuses_in_process_jq_environment_reads_before_gh_runs() {
    let harness = Harness::new();
    for argv in [
        vec![
            "pr",
            "view",
            "17",
            "--json",
            "title",
            "--jq",
            "env.GH_TOKEN | @base64",
        ],
        vec!["api", "user", "-q", "$ENV.GH_TOKEN | explode"],
        vec!["api", "user", "-qenv.GH_TOKEN"],
    ] {
        let reply = harness.call(&argv);
        assert_eq!(reply.exit_code, 1, "accepted {argv:?}");
        assert!(!harness.cwd.path().join("runs").exists());
    }
}

#[test]
fn refuses_writes_and_flag_bypasses_before_gh_runs() {
    let harness = Harness::new();
    let cases: &[&[&str]] = &[
        &["pr", "create"],
        &["pr", "merge", "17"],
        &["pr", "review", "17", "--approve"],
        &["pr", "close", "17"],
        &["pr", "edit", "17"],
        &["pr", "checkout", "17"],
        &["issue", "comment", "12", "--body", "hello"],
        &["issue", "create"],
        &["issue", "delete", "12", "--yes"],
        &["repo", "create"],
        &["repo", "clone", "o/r"],
        &["run", "rerun", "123"],
        &["run", "cancel", "123"],
        &["workflow", "run", "ci"],
        &["auth", "login"],
        &["auth", "token"],
        &["auth", "status"],
        &["alias", "set", "pr", "!echo bad"],
        &["extension", "exec", "write"],
        &["api", "user", "-X", "POST"],
        &["api", "user", "-X", "PATCH"],
        &["api", "user", "-X", "DELETE"],
        &["api", "user", "--method", "PUT"],
        &["api", "user", "--method=POST"],
        &["api", "user", "-XPOST"],
        &["api", "user", "-X=PATCH"],
        &["api", "user", "-iXDELETE"],
        &["api", "-XPOST", "user"],
        &["api", "user", "-XGET", "-XPOST"],
        &["api", "user", "-XPOST", "-XGET"],
        &["api", "user", "--meth=POST"],
        &["api", "user", "-f", "name=x"],
        &["api", "user", "-fname=x"],
        &["api", "user", "-F", "name=x"],
        &["api", "user", "-Fname=x"],
        &["api", "user", "--raw-field=name=x"],
        &["api", "user", "--field", "name=x"],
        &["api", "user", "--method=GET", "-fname=x"],
        &["api", "user", "--input", "-"],
        &["api", "user", "--input=-"],
        &["api", "user", "--method=GET", "--input", "body.json"],
        &["api", "user", "-H", "X-HTTP-Method-Override: POST"],
        &["api", "user", "--header", "Authorization: Bearer other"],
        &[
            "api",
            "user",
            "-H",
            "Accept: application/json\r\nX-HTTP-Method-Override: POST",
        ],
        &["api", "user", "--hostname", "evil.example"],
        &["api", "https://evil.example/user"],
        &["api", "//evil.example/user"],
        &["api", "http://api.github.com/user"],
        &["api", "https://api.github.com:444/user"],
        &["api", "https://name:password@api.github.com/user"],
        &["api", "graphql"],
        &["api", "graphql?query=mutation%20%7B%7D"],
        &["pr", "view", "17", "--web"],
        &["repo", "view", "owner/repo", "-w"],
        &["run", "view", "123", "--web"],
        &["issue", "view", "12", "--web"],
        &["pr", "view", "https://evil.example/o/r/pull/17"],
        &["pr", "view", "17", "-R", "evil.example/owner/repo"],
        &[
            "pr",
            "view",
            "17",
            "-Rowner/repo",
            "-Revil.example/owner/repo",
        ],
        &["repo", "view", "https://evil.example/owner/repo"],
        &["pr", "view", "17", "--future-write-flag"],
        &["pr", "view", "17", "extra"],
        &["api", "user", "--", "-XPOST"],
        &["api", "user", "--method"],
        &["api", "user", "--jq", "-XPOST"],
        &["pr", "VIEW", "17"],
        &["!write"],
        &[],
    ];
    for argv in cases {
        let reply = harness.call(argv);
        assert_eq!(reply.exit_code, 1, "accepted {argv:?}");
        assert!(reply.stdout.is_empty(), "{argv:?}");
        assert!(
            String::from_utf8_lossy(&reply.stderr).contains("gh broker:"),
            "{argv:?}"
        );
        assert!(
            !harness.cwd.path().join("runs").exists(),
            "gh ran for {argv:?}"
        );
    }
}

#[test]
fn refused_values_are_not_reflected_and_next_read_still_works() {
    let harness = Harness::new();
    let reply = harness.call(&["pr", "create", "--body", support::TOKEN]);
    assert!(!String::from_utf8_lossy(&reply.stderr).contains(support::TOKEN));
    assert!(!harness.cwd.path().join("runs").exists());
    assert_eq!(harness.call(&["pr", "list"]).exit_code, 0);
}
