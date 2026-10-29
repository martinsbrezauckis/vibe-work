# Public behavior tests

These tests exercise the product with invented data, disposable files and repositories, loopback
services, and the pinned real Vibe app server with a scripted model on loopback. Tests that never call Vibe start
no Vibe process. No live account, captured user data, release key or platform credential store is needed.
Vault passphrases and signing keys are generated in tests. Faults use the real process, its files or the model's HTTP
responses; the recording tap copies protocol bytes unchanged.

`rust/` is a root-workspace integration test crate. `ui/` renders production components in jsdom
through the typed fake core. Its shared `support/renderShell.tsx` and transcript rigs let future
tests render the real shell; `setup.ts` supplies the DOM facilities jsdom lacks.

| Behavior | Rust tests | UI tests |
| --- | --- | --- |
| Stale save refusal and preserved drafts | `files_write_stale` | `file-edit-draft` |
| PDF/workbook budgets and Unicode names | `documents_reading`, `handbook_unicode_names` | |
| Temporary-chat ledger durability and failure reporting | `temporary_session_ledger` | |
| Approval ownership, provenance, approve/deny and revocation | `approval_provenance`, `confirmation_tokens`, `session_delete_approval`, `ui_control_origin` | |
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
uv venv --python 3.14.8 data/vibe-3.0.3/venv
uv pip install --python data/vibe-3.0.3/venv --require-hashes --only-binary :all: -r crates/vw-vibe-install/pin/vibe.requirements.txt
cargo test -p vw-public-tests --no-fail-fast
pnpm --filter vibe-work-ui test
scripts/public/check.sh
```

For plain `cargo test` in a nested checkout, set `VW_TEST_ROOT` to an absolute folder outside Git (created if missing); `check.sh` chooses and removes a fresh one automatically when the checkout's parent is a Git work tree.

The UI commands create an ignored dependency link to `ui/node_modules` (a junction on Windows).
Typecheck, lint and formatting include the public UI tests. The public gate runs the Rust workspace
and UI tests on Linux, Windows and macOS through the existing CI matrix. CI installs the same hash-locked Vibe requirements as the developer setup and exports `VW_TEST_VIBE_SERVER` and
`VIBE_PYTHON` for real-server tests and code generation. The model and registry fixtures listen only on loopback;
placeholder credentials never reach a provider account.

Windows keeps explicit ignored cases in `platform_skips` for Unix shell/process groups, permission denial and
symlink/worktree behavior. An ignored case is missing coverage, not a pass.

These are source behavior tests. They do not prove packaged app startup, installation, native
webview interaction, signatures on shipped installers, or a public release. The separate unsigned
bundle workflow builds packages; it does not launch them.
