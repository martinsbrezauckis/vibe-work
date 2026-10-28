# Skills manager core

This crate depends on serde and typed errors. A future app adapter supplies its
transport and session id; the crate sends Vibe requests and projects returned
data without reading skills directories or owning a process.

`SkillsClient<T>` exposes the thirteen `skills/*` methods from Vibe 3.0.0.
`SkillsTransport::request` returns the unwrapped JSON-RPC result, or
`TransportError::Reply(ProtocolError)` for an error reply. Its future is `Send`.
The trait uses Rust's native return-position future, so no async-trait dependency
or runtime is required by the library.

```rust
use vw_skills::{SkillsClient, SkillsInstalledParams, SkillsTransport};

async fn read<T: SkillsTransport>(transport: T, session_id: String) {
    let client = SkillsClient::new(transport);
    let installed = client.installed(&SkillsInstalledParams { session_id }).await;
    // The app decides how to render success or SkillsError.
}
```

Mutation results preserve the entire typed runtime snapshot and its
`applied`/`pending` status. `convert_local` also returns Vibe's `converted` flag.
The future adapter applies the runtime to its session and refreshes the manager's
reads. The client does not add implicit calls after a mutation.

Vibe's import input is **a registry skill id**, with optional version/alias and
scope. The tagged service has no local-folder or URL import parameter. Strings,
versions, aliases and scopes pass through; validation remains on the server.
`remove` removes registry pins, and `convertLocal` adopts cached registry content
as a local skill. Neither method promises general local-file deletion/import.

`SkillsError` classifies protocol codes and preserves Vibe's reason and structured
details. It distinguishes input, conflict, access, unsupported-method, other
server, channel and wire errors. It does not parse reason strings or invent
folder validation messages.

`SkillsView::merge` takes installed, catalog, updates, the authoritative
`skills/list` response, and issues from a runtime snapshot:

- One row per name; every installed source/scope copy remains available for actions.
- The active copy comes from `skills/list`; a sole installed copy is the fallback.
  Multiple copies without a reported winner leave display values unset.
- Builtin/plugin source and global/project scope become display source labels.
  Original Vibe source/scope values remain on the installed and active copies.
- Catalog versions join by `skillId`. Update versions join by name/current
  version only when the installed identities are unambiguous. Signals remain
  visible even when they cannot be attributed to a particular copy.
- Update availability comes from Vibe's update signals. A catalog version alone
  does not create a new update signal or override Vibe's moving-alias behavior.
- Issues remain at view level: Vibe supplies a file and message, without a skill
  id or name. The projection does not infer ownership from file paths.

`skills/updates` marks new versions seen on the server. Retain its result for the
UI's notification lifetime instead of polling it as a persistent update status;
the catalog's `updates` map is the persistent signal for frozen pins.

Wire types and JSON fixtures come from the installed Pydantic models, including
nonempty default factories and the MCP display-name validator. Unknown extra
fields are ignored, nullable missing fields are `None`, declared defaults match
Vibe, and missing required fields or wrong known types fail deserialization.
Requests do not validate inputs a second time.

Regenerate/check with the pinned Vibe environment:

```sh
vibe_pin=$(cat tools/public/codegen/vibe.pin)
"${VIBE_PYTHON:-data/vibe-$vibe_pin/venv/bin/python}" crates/vw-skills/tools/generate.py --write
"${VIBE_PYTHON:-data/vibe-$vibe_pin/venv/bin/python}" crates/vw-skills/tools/generate.py --check
```

The generator checks that the interpreter has **mistral-vibe 3.0.2** installed. Fixture requests/results are model-written examples, not captured user
sessions. The schema fixtures preserve upstream JSON Schema for contract tests;
JSON fixtures use valid JSON without comment headers.

```sh
cargo test -p vw-skills --test replay --test wire --test errors --test view
cargo clippy -p vw-skills --all-targets -- -D warnings
cargo fmt --all --check
VW_SKILLS_LIVE=1 cargo test -p vw-skills --test live -- --nocapture
```

The opt-in live test uses `VIBE_PYTHON`, otherwise `data/vibe-<vibe.pin>/venv/bin/python`
(`Scripts/python.exe` on Windows), reading the version from `tools/public/codegen/vibe.pin`. It starts the actual
installed server with a disposable home/project, a keyless local model
configuration, no inherited environment, and an audit hook rejecting Internet
sockets. It submits no model turn. Without the switch it reports `SKIPPED`.
