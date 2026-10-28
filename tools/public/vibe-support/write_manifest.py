"""Writes the supported-Vibe manifest: the Vibe versions Vibe Work has tested, published beside the app
update (`latest.json`) on the same release channel and signed with the same updater key
(`tools/public/vibe-support/sign`, run by the release step). The app reads it with every update check
(crate `vw-vibe-install`, module `support`) and offers "Vibe Y available -> Update" for a tested version newer than
the running one; a newer Vibe on PyPI that is not listed only gets "support arrives in an upcoming update".

Inputs: one pin folder per tested version, as `tools/public/vibe-install/generate_pin.py --out <dir>` writes it
(`vibe-install.json` + `vibe.requirements.txt`; the pin embedded in the app is `crates/vw-vibe-install/pin`).
Each entry also says which Vibe Work versions it was tested with (`min=`, optional `max=`, inclusive): a Vibe that
needs newer generated types must never reach an older app.

Output (`--out`): `vibe-support.json` and `vibe-<version>.requirements.txt` per entry (the requirements file the
app downloads at install time and checks against the SHA-256 in the signed manifest). Every listed version's
requirements file must be uploaded with the manifest, to the same release.

Usage: python3 -I write_manifest.py --out <dir> --entry <pin dir>,min=0.1.0[,max=0.1.9] [--entry ...]
         [--pypi-latest 2.27.0 | --pypi-latest-from-pypi] [--sequence N] [--previous <old vibe-support.json>]
  --sequence   default: the current Unix time; must be higher than --previous's (the app refuses a lower one).
  --pypi-latest-from-pypi  reads `info.version` of https://pypi.org/pypi/mistral-vibe/json (network).
Needs: Python 3.10+, stdlib only. Called by: the release step, maintainers, the tests.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import re
import sys
import time
import urllib.request
from pathlib import Path

SCHEMA = 1
PLAIN = re.compile(r"^[0-9]+(\.[0-9]+)*$")
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


# Published Vibe versions no Vibe Work may install, with the reason the refusal names.
NEVER_OFFER = {
    "3.0.1": "it cannot open sessions that Vibe 3.0.0 saved during a tool call (fixed in 3.0.2)",
}


class ManifestError(Exception):
    """A refused input (the message says which and why)."""


def version_key(version: str) -> tuple[int, ...]:
    return tuple(int(part) for part in version.split("."))


def parse_entry(spec: str) -> tuple[Path, str, str | None]:
    """`<pin dir>,min=X[,max=Y]` -> (dir, min, max)."""
    folder, *options = spec.split(",")
    bounds: dict[str, str] = {}
    for option in options:
        key, _, value = option.partition("=")
        if key not in ("min", "max") or not value:
            raise ManifestError(f"--entry {spec!r}: options are min=<version> and max=<version>")
        if not SEMVER.match(value):
            raise ManifestError(f"--entry {spec!r}: {key} must be a Vibe Work version like 0.1.0")
        bounds[key] = value
    if "min" not in bounds:
        raise ManifestError(f"--entry {spec!r}: min=<Vibe Work version it was tested with> is required")
    return Path(folder), bounds["min"], bounds.get("max")


def load_entry(folder: Path, minimum: str, maximum: str | None, out: Path) -> dict:
    """Reads one pin folder, checks it, copies its requirements into `out`, returns the manifest entry."""
    pin = json.loads((folder / "vibe-install.json").read_text(encoding="utf-8"))
    vibe = pin["vibe"]
    version = vibe["version"]
    if not PLAIN.match(version):
        raise ManifestError(f"{folder}: version {version!r} is not a plain version")
    # The runtime/protocol migration is one-way: older apps cannot launch Vibe 3.
    app_min = version_key(minimum.split("-")[0].split("+")[0])
    app_max = version_key(maximum.split("-")[0].split("+")[0]) if maximum else None
    if version in NEVER_OFFER:
        raise ManifestError(f"Vibe {version} is never offered: {NEVER_OFFER[version]}")
    if version_key(version) >= (3, 0, 0) and app_min < (0, 1, 2):
        raise ManifestError("Vibe 3 needs min=0.1.2 or later")
    if version == "2.26.1" and (app_max is None or app_max > (0, 1, 1)):
        raise ManifestError("Vibe 2.26.1 needs max=0.1.1 or earlier")
    if app_max is not None and app_max < app_min:
        raise ManifestError("max must not be earlier than min")
    requirements = (folder / vibe.get("requirements", "vibe.requirements.txt")).read_bytes()
    digest = hashlib.sha256(requirements).hexdigest()
    if digest != vibe["requirementsSha256"]:
        raise ManifestError(f"{folder}: the requirements file does not match the pin ({digest})")
    if f"mistral-vibe=={version}" not in requirements.decode("utf-8"):
        raise ManifestError(f"{folder}: the requirements file does not pin mistral-vibe=={version}")
    wheels = {}
    for target, wheel in sorted(vibe["wheels"].items()):
        if not HEX64.match(wheel["sha256"]):
            raise ManifestError(f"{folder}: the {target} wheel has no SHA-256")
        wheels[target] = {"filename": wheel["filename"], "bytes": wheel["bytes"], "sha256": wheel["sha256"]}
    name = f"vibe-{version}.requirements.txt"
    (out / name).write_bytes(requirements)
    return {
        "version": version,
        "python": vibe["python"],
        "minVibeWork": minimum,
        "maxVibeWork": maximum,
        "requirements": name,
        "requirementsSha256": digest,
        "requirementsBytes": len(requirements),
        "packages": vibe["packages"],
        "wheels": wheels,
    }


def pypi_latest() -> str:
    request = urllib.request.Request("https://pypi.org/pypi/mistral-vibe/json",
                                     headers={"User-Agent": "vibe-work-manifest-writer"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)["info"]["version"]


def build(out: Path, entries: list[str], sequence: int | None, previous: Path | None,
          latest: str | None) -> dict:
    """Writes the manifest and the requirements files into `out`; returns the manifest."""
    out.mkdir(parents=True, exist_ok=True)
    floor = 0
    if previous is not None:
        floor = int(json.loads(previous.read_text(encoding="utf-8"))["sequence"])
    if sequence is None:
        sequence = max(int(time.time()), floor + 1)
    if sequence <= floor:
        raise ManifestError(f"--sequence {sequence} must be higher than the previous manifest's {floor}")
    if latest is not None and not PLAIN.match(latest):
        raise ManifestError(f"--pypi-latest {latest!r} is not a plain version")
    versions = [load_entry(*parse_entry(spec), out) for spec in entries]
    seen = [v["version"] for v in versions]
    if len(set(seen)) != len(seen):
        raise ManifestError(f"a version is listed twice: {seen}")
    versions.sort(key=lambda v: version_key(v["version"]), reverse=True)
    manifest = {
        "schema": SCHEMA,
        "sequence": sequence,
        "published": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "pypiLatest": latest,
        "versions": versions,
    }
    (out / "vibe-support.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8", newline="\n")
    return manifest


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--entry", action="append", required=True)
    parser.add_argument("--sequence", type=int)
    parser.add_argument("--previous", type=Path)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--pypi-latest")
    group.add_argument("--pypi-latest-from-pypi", action="store_true")
    args = parser.parse_args(argv)
    latest = pypi_latest() if args.pypi_latest_from_pypi else args.pypi_latest
    try:
        manifest = build(args.out, args.entry, args.sequence, args.previous, latest)
    except ManifestError as error:
        print(f"refused: {error}", file=sys.stderr)
        return 1
    listed = ", ".join(f"{v['version']} (Vibe Work {v['minVibeWork']}+)" for v in manifest["versions"])
    print(f"sequence {manifest['sequence']}; pypiLatest {manifest['pypiLatest']}; versions: {listed}")
    for name in sorted(p.name for p in args.out.iterdir()):
        print(f"  {args.out / name}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
