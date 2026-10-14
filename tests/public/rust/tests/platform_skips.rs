//! Explicit test skips for behavior that needs Unix shell or filesystem semantics.
//! The matching active tests live in the named integration test binaries on Unix.

#[cfg(not(unix))]
#[test]
#[ignore = "forged and revoked grants are exercised by a Unix shell script"]
fn forged_or_revoked_grant_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "locked grant lookup is exercised by a Unix shell script and process group"]
fn locked_grant_lookup_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "revoke-during-dialog test runs a Unix shell script"]
fn revoke_during_grant_dialog_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "secret broker integration drives Unix shell commands and process groups"]
fn secret_broker_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "rollback record save refusal depends on Unix file permissions"]
fn support_record_permission_refusal_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "temporary-file symlink guard depends on Unix symlink semantics"]
fn support_record_symlink_guard_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "file-write symlink escape test uses Unix symlink creation"]
fn file_write_symlink_escape_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "session file-scope symlink confinement test uses Unix symlink creation"]
fn session_file_symlink_confinement_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "worktree inspection compares git's worktree paths with Unix-style temporary paths"]
fn worktree_inspection_needs_unix() {}

#[cfg(not(unix))]
#[test]
#[ignore = "worktree session cleanup waits on worktree paths in their Unix form"]
fn worktree_session_cleanup_needs_unix() {}
