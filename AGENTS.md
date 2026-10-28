# AGENTS.md

How to change Vibe Work's own source code, for coding agents and people alike. Vibe Work is a desktop app around
Mistral's Vibe agent: Rust in `crates/` and `app/` (Tauri 2), React and TypeScript in `ui/`. `CONTRIBUTING.md` has the
code style; `README.md` (Develop) has the full setup.

## A fresh checkout or copy

- Run `pnpm install --frozen-lockfile` at the repository root first. Without `ui/node_modules` every UI check fails
  for a reason that has nothing to do with your change.
- Rust comes from `rust-toolchain.toml` (rustup installs it). The generated-code check needs the pinned Vibe in
  `data/vibe-<version>/venv`, the version in `tools/public/codegen/vibe.pin`, or `$VIBE_PYTHON` (README, Develop).

## Done means checked

- `scripts/public/check.sh` is the check every pull request must pass: formatting, a production UI build, Clippy
  with warnings as errors, Rust and UI tests, UI lint, type check and format, generated-code and vocabulary drift
  checks, and size, colour and public-paths guards. It runs every step, then ends with `check.sh PASSED in <n>s` or
  the list of failed steps. A cold run takes many minutes: give it a long timeout (or run it in the background) and
  report that last line as it is.
- While you work, run the part you changed:
  - UI: `pnpm --filter vibe-work-ui typecheck`, `pnpm --filter vibe-work-ui test`, `pnpm --filter vibe-work-ui lint`
  - Rust: `cargo fmt --all`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo test -p <crate>`
- Never say the tests pass without their output. If a check could not run, say which and why.

## Generated files: never edit them by hand

- `crates/vw-protocol/src/generated/` and `ui/src/protocol/generated/`: Vibe's protocol types. Regenerate with
  `data/vibe-<version>/venv/bin/python tools/public/codegen/generate.py --write` (or `$VIBE_PYTHON`).
- `ui/src/protocol/core/`: the core API types, made from the Rust types in `crates/vw-core-api`. Change the Rust type,
  then run `cargo run -p vw-core-api --features ts --bin gen-core-ts -- --write` (the first build takes minutes).
- A file whose header says GENERATED or "do not edit" is regenerated, never edited.

## Where code goes

- Logic lives in `crates/` and is tested without a window; `app/` only wires Tauri commands and events to it.
- The UI talks to the core only through the typed command client; components render, stores hold state and logic.
- Every file starts with a short header (what it does, what it depends on, who calls it). Files stay small: the size
  check enforces a ceiling.

## Git

- Work on your own branch. Do not rewrite commits that are already pushed; fix them with a new commit.
- When a check fails on a pull request, read its log first and fix what it names.
