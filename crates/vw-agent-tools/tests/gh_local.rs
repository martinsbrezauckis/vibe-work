//! Local GitHub CLI help stays available without granting access to an account; a redirect never turns a
//! GitHub write into a read.
use vw_agent_tools::gitparse::scan;

#[test]
fn local_help_and_version_allow_redirects_but_never_remote_commands() {
    for command in [
        "gh --version",
        "gh --version 2>/dev/null",
        "gh --version 2> /dev/null",
        "gh --version >/dev/null 2>&1",
        "gh --version 2>/dev/null | head -1",
        "gh help",
        "gh help pr >help.txt",
        "gh help 2>&1",
        "gh --version >>version.txt",
        "gh --version >$VW_LOCAL_HELP",
        "gh help 2>$VW_HELP",
        r#"gh --version > "$VW_LOCAL_HELP""#,
        r#"gh help >> "${VW_HELP}" 2>&1"#,
        "gh version",
        "gh version 2>/dev/null",
        "gh --help",
        "gh --help 2>/dev/null",
        "gh -h >/dev/null",
    ] {
        assert!(
            !scan(command).needs_core(),
            "local command refused: {command}"
        );
    }
    for command in [
        "gh pr view 12",
        "gh pr view 12 2>/dev/null",
        "gh pr merge 12",
        "gh pr merge 12 2>/dev/null",
        "gh pr merge 12 >/dev/null 2>&1",
        "gh pr 2>/dev/null merge 12",
        "gh api -X POST repos/o/r/issues >/dev/null",
        "gh auth token",
        "gh auth token 2>/dev/null",
        "gh --version; gh pr merge 12",
        "gh version && gh pr merge 12",
        "gh help $(gh auth token)",
        "gh --version >$(gh auth token)",
        "gh --help >`gh auth token`",
        "gh version extra",
    ] {
        assert!(
            scan(command).needs_core(),
            "remote command escaped: {command}"
        );
    }
}

#[test]
fn a_redirect_is_not_part_of_the_command_named_in_a_refusal() {
    for (command, named) in [
        ("gh pr view 12 2>/dev/null", "gh pr view"),
        ("gh pr 2>/dev/null merge 12", "gh pr merge"),
        ("gh 2>/dev/null pr merge 12 >out.txt", "gh pr merge"),
    ] {
        let found = scan(command);
        assert!(
            found.ops.iter().all(|op| op.what == named),
            "{command}: {:?}",
            found
                .ops
                .iter()
                .map(|op| op.what.as_str())
                .collect::<Vec<_>>()
        );
    }
}
