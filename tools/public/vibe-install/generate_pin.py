"""Generates the pinned install data for Vibe: what Vibe Work downloads, from where, with which
checksums, when it installs Vibe for the user at setup (crate `vw-vibe-install`).

Vibe Work ships no Vibe. With the user's consent it installs, into an app-private folder:
- uv, from Astral's official GitHub release, checked against the release's SHA-256;
- a Python through that uv;
- `mistral-vibe==<version>` from PyPI with `--require-hashes`, from a requirements file generated here from Vibe's
  own `uv.lock` at the pinned tag (every dependency pinned and hashed), plus the hashes of Mistral's published
  wheels for `mistral-vibe` itself.

Sources (all official, all fetched at generation time and written into the data with their checksums):
- the tag: `git clone --depth 1 --branch v<version>` of github.com/mistralai/mistral-vibe, commit checked against
  `--commit`; `uv export --frozen --no-dev --no-emit-project` of its lock;
- PyPI's JSON API for `mistral-vibe/<version>` (wheel names, sizes, SHA-256);
- GitHub's release API for `astral-sh/uv` `<uv version>` (asset sizes and digests), cross-checked with the release's
  own `<asset>.sha256` files (two independent statements of the same checksum must agree).
Consistency checks (generation fails otherwise): the export names exactly the packages and versions that the
published wheel's `Requires-Dist` pins; a wheel exists for every target we ship.

Usage: python3 -I generate_pin.py --vibe 3.0.3 --commit <sha> --uv 0.12.24 --python 3.14.8 --out <crate>/pin [--work <scratch>]
Needs: git, uv, network. Writes `vibe-install.json` and `vibe.requirements.txt` into --out.
Called by: maintainers when bumping Vibe (with tools/public/codegen/vibe.pin, regenerated types, bench, gate).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import tomllib
import urllib.request
from pathlib import Path

VIBE_REPO = "https://github.com/mistralai/mistral-vibe.git"
# Targets Vibe Work ships, as (Rust target triple, uv asset suffix, wheel platform tag).
TARGETS = {
    "x86_64-unknown-linux-gnu": ("x86_64-unknown-linux-gnu.tar.gz", "manylinux_2_28_x86_64"),
    "aarch64-apple-darwin": ("aarch64-apple-darwin.tar.gz", "macosx_11_0_arm64"),
    "x86_64-pc-windows-msvc": ("x86_64-pc-windows-msvc.zip", "win_amd64"),
}

def check_python(python: str, required: str, lock_required: str) -> None:
    """Require an exact patch and matching published/locked Python floors; unknown specifiers fail closed."""
    if not re.fullmatch(r"3\.\d+\.\d+", python):
        raise SystemExit("--python must pin an exact CPython patch, such as 3.14.8")
    floor = re.fullmatch(r">=(\d+)\.(\d+)(?:\.(\d+))?", required)
    if not floor or lock_required != required:
        raise SystemExit(f"review Python constraints: wheel {required!r}, lock {lock_required!r}")
    minimum = tuple(int(v or 0) for v in floor.groups())
    if tuple(map(int, python.split("."))) < minimum:
        raise SystemExit(f"Python {python} does not satisfy {required}")


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "vibe-work-pin-generator"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def norm(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def exported(tag_dir: Path) -> str:
    result = subprocess.run(["uv", "export", "--frozen", "--no-dev", "--no-emit-project", "--format",
                             "requirements-txt", "--no-header"], cwd=tag_dir, check=True, capture_output=True, text=True)
    return result.stdout


def check_consistency(export: str, requires_dist: list[str]) -> None:
    pinned = {}
    for item in requires_dist:
        match = re.match(r"([A-Za-z0-9_.\-]+)(?:\[[^\]]*\])?\s*==\s*([^\s;]+)", item)
        if match:
            pinned[norm(match.group(1))] = match.group(2)
        elif "extra ==" not in item:
            raise SystemExit(f"the wheel has an unpinned requirement outside an extra: {item}")
    locked = {norm(m.group(1)): m.group(2) for m in re.finditer(r"^([a-z0-9_.\-]+)==([^\s;]+)", export, re.M)}
    if pinned != locked:
        raise SystemExit(f"lock export and wheel Requires-Dist differ: only wheel {sorted(set(pinned) - set(locked))}, "
                         f"only lock {sorted(set(locked) - set(pinned))}, "
                         f"versions {[k for k in pinned.keys() & locked.keys() if pinned[k] != locked[k]]}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vibe", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--uv", required=True)
    parser.add_argument("--python", required=True)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--work", type=Path)
    args = parser.parse_args()
    work = args.work or Path(tempfile.mkdtemp(prefix="vibe-pin-"))
    tag_dir = work / f"mistral-vibe-{args.vibe}"
    if not tag_dir.exists():
        subprocess.run(["git", "-c", "core.autocrlf=false", "clone", "-q", "--depth", "1", "--branch", f"v{args.vibe}",
                        VIBE_REPO, str(tag_dir)], check=True)
    head = subprocess.run(["git", "-C", str(tag_dir), "rev-parse", "HEAD"], check=True, capture_output=True,
                          text=True).stdout.strip()
    if head != args.commit:
        raise SystemExit(f"tag v{args.vibe} is {head}, expected {args.commit}")

    pypi = json.loads(fetch(f"https://pypi.org/pypi/mistral-vibe/{args.vibe}/json"))
    lock = tomllib.loads((tag_dir / "uv.lock").read_text(encoding="utf-8"))
    check_python(args.python, pypi["info"]["requires_python"], lock["requires-python"])
    uv_version = subprocess.check_output(["uv", "--version"], text=True).split()[1]
    if uv_version != args.uv:
        raise SystemExit(f"run the generator with uv {args.uv}, found {uv_version}")
    export = exported(tag_dir)
    check_consistency(export, pypi["info"]["requires_dist"] or [])
    wheels = {}
    for target, (_asset, platform_tag) in TARGETS.items():
        found = [u for u in pypi["urls"] if u["packagetype"] == "bdist_wheel" and u["filename"].endswith(f"{platform_tag}.whl")]
        if len(found) != 1:
            raise SystemExit(f"expected one mistral-vibe wheel for {platform_tag}, found {len(found)}")
        wheels[target] = {"filename": found[0]["filename"], "bytes": found[0]["size"],
                          "sha256": found[0]["digests"]["sha256"], "url": found[0]["url"]}
    hashes = " \\\n".join(f"    --hash=sha256:{w['sha256']}" for w in sorted(wheels.values(), key=lambda w: w["filename"]))
    requirements = (f"# Generated by tools/public/vibe-install/generate_pin.py: mistral-vibe {args.vibe} (tag v{args.vibe},"
                    f" commit {args.commit}).\n# Dependencies: `uv export --frozen --no-dev --no-emit-project` of the tag's"
                    f" uv.lock; mistral-vibe: Mistral's published wheels on PyPI.\n# Installed with --require-hashes"
                    f" --only-binary :all:. Do not edit by hand.\nmistral-vibe=={args.vibe} \\\n{hashes}\n{export}")

    release = json.loads(fetch(f"https://api.github.com/repos/astral-sh/uv/releases/tags/{args.uv}"))
    by_name = {a["name"]: a for a in release["assets"]}
    uv_assets = {}
    for target, (suffix, _tag) in TARGETS.items():
        name = f"uv-{suffix}"
        asset, sums = by_name.get(name), by_name.get(f"{name}.sha256")
        if not asset or not sums:
            raise SystemExit(f"uv {args.uv} has no {name} (+ .sha256)")
        stated = fetch(sums["browser_download_url"]).decode().split()[0].lower()
        digest = (asset.get("digest") or "").removeprefix("sha256:").lower()
        if not re.fullmatch(r"[0-9a-f]{64}", stated) or stated != digest:
            raise SystemExit(f"{name}: the .sha256 file ({stated}) and the release digest ({digest}) disagree")
        uv_assets[target] = {"url": asset["browser_download_url"], "bytes": asset["size"], "sha256": stated,
                             "archive": "zip" if name.endswith(".zip") else "tar.gz"}

    req_name = "vibe.requirements.txt"  # fixed name: the crate embeds it with include_str!
    data = {
        "_comment": "Generated by tools/public/vibe-install/generate_pin.py. Read by crates/vw-vibe-install (embedded). "
                    "Bumping Vibe = regenerate this with the new tag, plus tools/public/codegen/vibe.pin, types, bench, gate.",
        "vibe": {"version": args.vibe, "tag": f"v{args.vibe}", "commit": args.commit, "python": args.python,
                 "pypiPage": f"https://pypi.org/project/mistral-vibe/{args.vibe}/",
                 "docs": "https://github.com/mistralai/mistral-vibe#readme", "license": "Apache-2.0",
                 "requirements": req_name, "requirementsSha256": hashlib.sha256(requirements.encode()).hexdigest(),
                 "packages": len(re.findall(r"^[a-z0-9_.\-]+==", export, re.M)) + 1, "wheels": wheels},
        "uv": {"version": args.uv, "releasePage": f"https://github.com/astral-sh/uv/releases/tag/{args.uv}",
               "assets": uv_assets},
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / req_name).write_text(requirements, encoding="utf-8", newline="\n")
    (args.out / "vibe-install.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"vibe": args.vibe, "uv": args.uv, "packages": data["vibe"]["packages"],
                      "requirementsSha256": data["vibe"]["requirementsSha256"]}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
