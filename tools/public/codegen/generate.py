#!/usr/bin/env python3
"""Generate the Rust and TypeScript protocol types from the installed Mistral Vibe package.

Role: single entry point of the generator. Run it with the interpreter that has Vibe
installed (data/vibe-3.0.3/venv/bin/python, or the one named by $VIBE_PYTHON).
  --write   regenerate crates/vw-protocol/src/generated and ui/src/protocol/generated
  --check   regenerate into a scratch directory and fail on any difference (CI drift gate)
Fails loudly when: the installed Vibe version differs from vibe.pin; method_map.json is out of
step with protocol.SERVER_METHODS or names a model that does not exist; a model uses a construct
the generator does not support; the IR disagrees with Pydantic's own JSON Schema (aliases,
required-ness) for any model.
Depends on: introspect.py, areas.py, rust_emit.py, ts_emit.py, registry_emit.py; rustfmt and
prettier (via pnpm) for formatting. Called by: scripts/public/check.sh and developers.
"""

from __future__ import annotations

import argparse
import difflib
import importlib.metadata
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import registry_emit  # noqa: E402 - script directory is added above for isolated Python imports.
import rust_emit  # noqa: E402 - script directory is added above for isolated Python imports.
import ts_emit  # noqa: E402 - script directory is added above for isolated Python imports.
from areas import split_files  # noqa: E402 - script directory is added above for isolated Python imports.
from introspect import GenerationError, Introspector, build_schema  # noqa: E402 - script directory is added above for isolated Python imports.
from ir import ModelDef, Schema  # noqa: E402 - script directory is added above for isolated Python imports.

ROOT = HERE.parents[2]
RUST_DIR = Path("crates/vw-protocol/src/generated")
TS_DIR = Path("ui/src/protocol/generated")


def check_pin_and_map(request_map: dict) -> str:
    """Verify the Vibe version pin and that method_map.json matches protocol.SERVER_METHODS."""
    pin = (HERE / "vibe.pin").read_text(encoding="utf-8").strip()
    installed = importlib.metadata.version("mistral-vibe")
    if installed != pin:
        raise GenerationError(f"Vibe {installed} is installed but the generator is pinned to {pin}")
    from vibe.app_server import protocol

    declared = set(protocol.SERVER_METHODS)
    mapped = set(request_map["requests"]) - {"initialize"}
    if declared != mapped:
        raise GenerationError(
            f"method_map.json drifted from protocol.SERVER_METHODS: "
            f"unmapped={sorted(declared - mapped)} stale={sorted(mapped - declared)}"
        )
    return installed


def verify_against_json_schema(intro: Introspector, schema: Schema) -> None:
    """Cross-check every model against Pydantic's own JSON Schema: same keys, same required set."""
    from pydantic import TypeAdapter

    for cls, name in intro.classes.items():
        d = schema.defs[name]
        assert isinstance(d, ModelDef)
        js = TypeAdapter(cls).json_schema(by_alias=True, mode="serialization")
        props = set(js.get("properties", {}))
        mine = {f.wire for f in d.fields}
        if props != mine:
            raise GenerationError(f"{name}: wire keys differ from Pydantic: {sorted(props ^ mine)}")
        js_val = TypeAdapter(cls).json_schema(by_alias=True, mode="validation")
        # Union tags are forced required by the generator, so they are excluded on both sides.
        tags = {f.wire for f in d.fields if f.discriminator}
        required = set(js_val.get("required", [])) - tags
        mine_required = {f.wire for f in d.fields if f.required and not f.discriminator}
        if required != mine_required:
            raise GenerationError(f"{name}: required keys differ from Pydantic: {sorted(required ^ mine_required)}")


def render(schema: Schema) -> dict[Path, str]:
    """All generated files as {relative path: unformatted text}."""
    rust_emit.check_names(schema)
    files = split_files(schema)
    file_of = {n: stem for stem, names in files.items() for n in names}
    out: dict[Path, str] = {}
    for stem, names in files.items():
        out[RUST_DIR / f"{stem}.rs"] = rust_emit.emit_file(f"{stem} protocol types.", names, schema)
        out[TS_DIR / f"{stem}.ts"] = ts_emit.emit_file(stem, names, schema, file_of)
    out[RUST_DIR / "mod.rs"] = rust_emit.emit_mod_index(list(files), ["methods", "reencode"])
    out[RUST_DIR / "support.rs"] = (HERE / "templates/support.rs").read_text(encoding="utf-8")
    out[RUST_DIR / "methods.rs"] = registry_emit.emit_rust(schema)
    out[RUST_DIR / "reencode.rs"] = registry_emit.emit_reencode(schema)
    out[TS_DIR / "index.ts"] = ts_emit.emit_index(list(files), [])
    out[TS_DIR / "support.ts"] = (HERE / "templates/support.ts").read_text(encoding="utf-8")
    out[TS_DIR / "methods.ts"] = registry_emit.emit_ts(schema)
    return out


def _tool(name: str) -> str:
    """Absolute path of a formatter on PATH.

    On Windows `pnpm` is a `pnpm.cmd` shim, which `subprocess` cannot start by its bare name (it does
    not apply PATHEXT); `shutil.which` does. Fails loudly instead of silently skipping formatting.
    """
    found = shutil.which(name)
    if found is None:
        raise SystemExit(f"generate.py: '{name}' not found on PATH (needed to format generated code)")
    return found


def format_tree(base: Path, rel_paths: list[Path]) -> None:
    """Format generated files in place with the project's rustfmt and prettier settings."""
    rust = [str(base / p) for p in rel_paths if p.suffix == ".rs"]
    ts = [str(base / p) for p in rel_paths if p.suffix == ".ts"]
    subprocess.run(
        [_tool("rustfmt"), "--edition", "2024", "--config-path", str(ROOT / "rustfmt.toml"), *rust], check=True
    )
    subprocess.run(
        [_tool("pnpm"), "--silent", "--dir", str(ROOT / "ui"), "exec", "prettier", "--config", str(ROOT / "ui/.prettierrc.json"),
         "--write", "--log-level", "warn", *ts],
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()

    request_map = json.loads((HERE / "method_map.json").read_text(encoding="utf-8"))
    try:
        version = check_pin_and_map(request_map)
        intro = Introspector()
        schema = build_schema(request_map, intro)
        schema.vibe_version = version
        verify_against_json_schema(intro, schema)
        generated = render(schema)
    except GenerationError as exc:
        print(f"codegen FAILED: {exc}", file=sys.stderr)
        return 1

    stage = Path(tempfile.mkdtemp(prefix="vw-codegen-"))
    try:
        for rel, text in generated.items():
            (stage / rel).parent.mkdir(parents=True, exist_ok=True)
            (stage / rel).write_text(text, encoding="utf-8", newline="\n")
        format_tree(stage, list(generated))
        formatted = {rel: (stage / rel).read_text(encoding="utf-8") for rel in generated}
        if args.write:
            for directory in (RUST_DIR, TS_DIR):
                shutil.rmtree(ROOT / directory, ignore_errors=True)
                (ROOT / directory).mkdir(parents=True)
            for rel, text in formatted.items():
                (ROOT / rel).write_text(text, encoding="utf-8", newline="\n")
            print(f"wrote {len(formatted)} files for Vibe {version}")
            return 0
        problems = []
        for directory in (RUST_DIR, TS_DIR):
            on_disk = {p.relative_to(ROOT) for p in (ROOT / directory).glob("*")} if (ROOT / directory).exists() else set()
            expected = {rel for rel in formatted if rel.parent == directory}
            problems += [f"stale file: {p}" for p in sorted(on_disk - expected)]
        shown = []  # the first differing files, printed as a short unified diff so a CI log shows the cause
        for rel, text in formatted.items():
            path = ROOT / rel
            if not path.exists():
                problems.append(f"missing: {rel}")
            elif (on_disk_text := path.read_text(encoding="utf-8")) != text:
                problems.append(f"differs: {rel}")
                if len(shown) < 2:
                    shown.append((rel, on_disk_text, text))
        if problems:
            print("generated code is out of date (run generate.py --write):", *problems[:20], sep="\n  ", file=sys.stderr)
            for rel, old, new in shown:
                diff = difflib.unified_diff(
                    old.splitlines(), new.splitlines(), f"committed/{rel}", f"generated/{rel}", lineterm="", n=1
                )
                print(*list(diff)[:40], sep="\n", file=sys.stderr)
            return 1
        print(f"generated code is up to date ({len(formatted)} files, Vibe {version})")
        return 0
    finally:
        shutil.rmtree(stage, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
