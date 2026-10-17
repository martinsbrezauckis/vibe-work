# Public behavior tests

These tests exercise the product with invented data, disposable files and repositories, loopback
services, and a scripted fake Vibe. They need no live account, installed Vibe, captured user data,
release key or platform credential store. Vault passphrases and signing keys are generated in tests.

`rust/` is a root-workspace integration test crate. `ui/` renders production components in jsdom
through the typed fake core. Its shared `support/renderShell.tsx` and transcript rigs let future
tests render the real shell; `setup.ts` supplies the DOM facilities jsdom lacks.

| Behavior | Rust tests | UI tests |
| --- | --- | --- |
| Stale save refusal and preserved drafts | `files_write_stale` | `file-edit-draft` |
| PDF/workbook budgets and Unicode names | `documents_reading`, `handbook_unicode_names` | |
| Temporary-chat ledger durability and failure reporting | `temporary_session_ledger` | |
| Approval ownership, provenance, approve/deny and revocation | `approval_provenance`, `confirmation_tokens`, `session_delete_approval`, `ui_control_origin`, `command_grant_provenance` | |
| Vault user-only management, locked refusal and value redaction | `vault_user_commands`, `vault_key_origin`, `secret_broker` | |
| Session ownership and file confinement | `files_write_stale`, `session_file_scope` | |
| Refusing stale Changes reverts | `changes_keep_revert` | |
| Clean worktree removal and dirty-copy preservation/reporting | `worktree_inspection`, `worktree_session_cleanup` | |
| Rejecting invalid/missing update signatures | `update_flow`, `update_manifest`, `supported_vibe_manifest` | |
| Changing Subagents while the open chat keeps working | `subagents_setting` | `subagents-setting` |
| Agent browser address classes: only public addresses are reachable | `netguard_ip_classes` | |

The Debug API is a user-level interface for ordinary chat commands: callback answers retain their
actual origin. Vault management and secret values remain user-only operations in the app window.

From a fresh checkout:

```sh
pnpm install --frozen-lockfile
cargo test -p vw-public-tests --no-fail-fast
pnpm --filter vibe-work-ui test
scripts/public/check.sh
```

The UI commands create an ignored dependency link to `ui/node_modules` (a junction on Windows).
Typecheck, lint and formatting include the public UI tests. The public gate runs the Rust workspace
and UI tests on Linux, Windows and macOS through the existing CI matrix. Its generated-code check
also needs the pinned Vibe Python environment described in the root README.

Unix runs register 127 Rust behavior cases and 15 UI cases. Windows registers 105 active Rust
cases plus ten explicitly ignored cases in `platform_skips`: four use Unix shell/process groups,
one needs Unix permission denial, three need Unix symlink creation and two stand for the two
worktree test files, which compare git's worktree paths with Unix-style temporary paths. In mixed
file-scope cases, Unix-only symlink/permission assertions are cfg-gated; the portable path and
write assertions still run. An ignored case is missing coverage, not a pass.

These are source behavior tests. They do not prove packaged app startup, installation, native
webview interaction, signatures on shipped installers, or a public release. The separate unsigned
bundle workflow builds packages; it does not launch them.
