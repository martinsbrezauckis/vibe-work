# Public script registry

Every file in `scripts/public/` is listed here with its public purpose. A script not listed here
does not belong in this folder. Public scripts never import, run or name the private scripts folder.

| File | Purpose |
|---|---|
| `check.sh` | The one command every change must pass: rustfmt, UI bundle, clippy, cargo tests, UI typecheck, lint, format and tests, size check, colour check, public-paths scan, generated-code and vocabulary-reference drift checks. Collects failed steps and names them at the end. |
| `check_size.mjs` | Fails when a source file in `crates/`, `app/`, `ui/src/`, `tools/public/` or `scripts/public/` exceeds 800 lines; honours the no-regrowth ceilings in `size-grandfather.json`; prints the five largest files. |
| `size-grandfather.json` | Path to line-ceiling map for files allowed to stay over the limit without growing. Empty. |
| `check_colors.mjs` | Fails on any literal colour (hex, `rgb()`, named) in `ui/src/` outside `styles/tokens.css`, so every colour lives in the theme tokens. |
| `generate_icons.mjs` | Regenerates desktop PNG, ICO and ICNS assets from the source SVG with the pinned Tauri CLI, verifies BrandMark geometry and colours, fits every frame to integer cells and a 1 px tile edge, and removes its scratch files. |
| `check_public_paths.mjs` | Validates the export allowlist in `release/public-paths.json`: no forbidden marker strings inside allowlisted files (plus the maintainers' extra markers when their file exists), no allowlisted file under a never-export path, and every listed path exists. In the public repository (`--public`, `VW_PUBLIC_REPO=1`, or any checkout that does not look like the private source tree) every tracked file must also be inside the allowlist. |
| `lib_files.mjs` | Shared helper: lists project files (tracked plus untracked-not-ignored) for the two check scripts. |
| `README.md` | Folder description. |
| `REGISTRY.md` | This registry. |
