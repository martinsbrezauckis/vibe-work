# Contributing

Thank you for helping. Bug reports, ideas and pull requests are welcome.

## Reporting a bug

Open an issue with what you did, what you expected, what happened, and your system and Vibe Work version (Settings ›
About). A screenshot helps. For security problems, follow `SECURITY.md` instead of opening an issue.

## Asking for a feature

Post what you want Vibe Work to do in [Discussions](https://github.com/martinsbrezauckis/vibe-work/discussions),
category Ideas: the task, how you would use it, and an example. Requests there shape the next releases.

## Making a change

1. Set up a development build as described in `README.md` (Develop).
2. Keep the change focused on one thing. Logic belongs in `crates/`; `app/` only wires the Tauri commands and
   events to it.
3. Run `scripts/public/check.sh`. It runs formatting, a production UI build, Clippy with warnings as errors, Rust
   and UI tests, UI lint, type check and format, generated-code and vocabulary drift checks, and size, colour and
   public-paths guards. A pull request must pass it.
4. Open a pull request that says what changed, why, and how you tested it.

## Code style

- Every file starts with a short header: what it is responsible for, what it depends on, who calls it, and what it
  guarantees. Public items have doc comments; comments explain why, not what.
- Rust: typed errors, no `unwrap` or `expect` outside tests, no `unsafe` outside `vw-platform`, `rustfmt` and Clippy
  clean.
- TypeScript: strict mode, no `any`; components render, state and logic live in the stores; the UI talks to the core
  only through the typed command client.
- Files stay small (a size check enforces a ceiling).
- The protocol types in `crates/vw-protocol/src/generated/` and `ui/src/protocol/generated/` come from pinned Vibe
  3.0.3; `ui/src/protocol/core/` comes from Rust core API types. Change the generator or source types and regenerate
  as described in `AGENTS.md`; never edit generated files by hand.

## License

By contributing you agree that your contribution is licensed under the MIT License of this project (`LICENSE`).
