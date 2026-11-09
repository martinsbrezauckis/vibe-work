# Public script registry

Every file in `scripts/public/` is listed here with its public purpose. A script not listed here
does not belong in this folder. Public scripts never import, run or name the private scripts folder.

| File | Purpose |
|---|---|
| `check.sh` | The one command every change must pass: rustfmt, UI bundle, clippy, cargo tests, UI typecheck, lint, format and tests, size check, colour check, macOS minimum check, public-paths scan, generated-code and vocabulary-reference drift checks. Collects failed steps and names them at the end. |
| `check_size.mjs` | Fails when a source file in `crates/`, `app/`, `ui/src/`, `tools/public/`, `scripts/public/` or `tests/` exceeds 800 lines; honours the no-regrowth ceilings in `size-grandfather.json`; prints the five largest files. |
| `size-grandfather.json` | Path to line-ceiling map for files allowed to stay over the limit without growing. Empty. |
| `check_colors.mjs` | Fails on any literal colour (hex, `rgb()`, named) in `ui/src/` outside `styles/tokens.css`, so every colour lives in the theme tokens. |
| `check_macos_minimum.mjs` | Fails when the macOS app is built without the preview network guard (the app crate's default feature `macos-proxy`) or when `app/tauri.conf.json`, `.cargo/config.toml` and the README Install table disagree on the oldest supported macOS (at least 14, which the guard needs). |
| `check_linux_tls.mjs` | Rejects OpenSSL in the Linux app's normal dependency tree and, when a binary path is supplied, its ELF NEEDED entries. |
| `generate_icons.mjs` | Regenerates desktop PNG, ICO and ICNS assets from the source SVG with the pinned Tauri CLI, verifies BrandMark geometry and colours, fits every frame to integer cells and a 1 px tile edge, and removes its scratch files. |
| `check_public_paths.mjs` | Validates the export allowlist in `release/public-paths.json`: no forbidden marker strings inside allowlisted files (plus the maintainers' extra markers when their file exists), no allowlisted file under a never-export path, and every listed path exists. In the public repository (`--public`, `VW_PUBLIC_REPO=1`, or any checkout that does not look like the private source tree) every tracked file must also be inside the allowlist. |
| `lib_files.mjs` | Shared helper: lists project files (tracked plus untracked-not-ignored) for the two check scripts. |
| `README.md` | Folder description. |
| `REGISTRY.md` | This registry. |
| `check_first_paint.mjs` | Checks that startup assets stay within the byte budget and exclude the code editor. |
| `check_release_features.mjs` | Checks the shipping app’s resolved Cargo features on every supported target. |
| `check_ui_features.mjs` | Checks the production UI artifact against the app’s release features. |
| `link_ui_test_modules.mjs` | Links the public UI tests to the installed UI dependencies. |
| `lint_public_ui_tests.mjs` | Runs strict lint over the public UI tests. |
| `release_sidecar.mjs` | Builds the tool sidecar with the app’s release features. |
| `release_ui_env.mjs` | Builds the UI environment for release feature selection. |
