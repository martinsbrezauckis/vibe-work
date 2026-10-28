"""Adversarial reads against the pinned Vibe helpers embedded by vw-core-api.

Called directly or by the crate's tests; uses temporary plugin homes and the real
resolver. Checks rejection before parsing and observes filesystem write attempts.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]
SOURCE = REPO / "crates/vw-core-api/src"
SCHEMA = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
MIB = 1024 * 1024
TEXT_LIMIT = 8 * MIB
TREE_LIMIT = 300 * MIB
MANIFESTS = (
    "plugin.json", ".claude-plugin/plugin.json", ".codex-plugin/plugin.json",
    "kimi.plugin.json", ".kimi-plugin/plugin.json", "package.json",
    "opencode.json", "opencode.jsonc", ".opencode/opencode.json",
    ".opencode/opencode.jsonc", ".opencode/plugins/main.ts",
    ".opencode/tools/main.ts", ".opencode/skills/check/SKILL.md",
)
AUDIT = """
import os, sys
writes = []
def observe(event, args):
    if event == 'open' and args[2] & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC):
        writes.append((event, str(args[0])))
    if event in {'os.mkdir', 'os.rename', 'os.remove', 'os.rmdir', 'os.symlink', 'os.link'}:
        writes.append((event, str(args[0])))
sys.addaudithook(observe)
"""


class PluginReads(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vw-plugin-read-test-")
        self.addCleanup(self.temp.cleanup)
        # Resolved: Vibe reports and trusts resolved folders, and the macOS temporary folder lies below the /var link.
        self.base = Path(self.temp.name).resolve()
        self.home = self.base / "home"
        self.cwd = self.base / "project"
        self.home.mkdir()
        self.cwd.mkdir()
        self.folder = self.home / "plugins-off" / "candidate"
        self.folder.mkdir(parents=True)

    def write(self, relative, data):
        path = self.folder / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data.encode() if isinstance(data, str) else data)
        return path

    def native(self):
        self.write("plugin.json", json.dumps({"$schema": SCHEMA, "name": "candidate"}))

    def invoke(self, kind, audit=False, extra=""):
        guard = SOURCE / "plugin_read_guard.py"
        script = guard.read_text() if guard.exists() else ""
        script += "\n" + (SOURCE / f"plugin_{kind}.py").read_text()
        if audit:
            script = AUDIT + script + "\nassert not writes, f'Listing attempted filesystem writes: {writes}'\n"
        script = extra + "\n" + script
        args = [str(self.folder)] if kind == "identity" else [str(self.home), str(self.cwd), "{}"]
        result = subprocess.run(
            [sys.executable, "-I", "-B", "-c", script, *args],
            env={**os.environ, "VIBE_HOME": str(self.home)}, cwd=self.cwd,
            capture_output=True, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    def refused(self):
        with self.subTest(helper="identity"):
            self.assertEqual(self.invoke("identity"), [], "unsafe folder established identity")
        with self.subTest(helper="discovery"):
            result = self.invoke("discovery")
            rows = [p for p in result["plugins"] if p["path"] == str(self.folder)]
            self.assertEqual(len(rows), 1, result)
            self.assertFalse(rows[0]["available"])
            self.assertEqual(rows[0]["components"], [])
            self.assertTrue(any("not readable" in s for s in rows[0]["issues"]), rows[0])

    def test_oversized_manifest_for_every_detected_format(self):
        for manifest in MANIFESTS:
            with self.subTest(manifest=manifest):
                # Even a lower-precedence marker must be checked before native resolution.
                self.native()
                path = self.write(manifest, json.dumps({"$schema": SCHEMA, "name": "candidate"}) + " " * TEXT_LIMIT)
                self.refused()
                path.unlink()

    def test_standalone_formats_accept_bounded_and_reject_oversized_manifests(self):
        for manifest in MANIFESTS:
            with self.subTest(manifest=manifest):
                shutil.rmtree(self.folder)
                self.folder.mkdir()
                data = json.dumps({"name": "candidate"})
                if manifest == "plugin.json":
                    data = json.dumps({"$schema": SCHEMA, "name": "candidate"})
                elif manifest == "package.json" or "opencode" in manifest:
                    self.write(".opencode/skills/check/SKILL.md",
                               "---\nname: check\ndescription: Check changes\n---\nCheck changes.")
                    if manifest.endswith("SKILL.md"):
                        data = (self.folder / manifest).read_text()
                    elif manifest.endswith(".ts"):
                        data = "export default {};"
                    elif manifest != "package.json":
                        data = "{}"
                # Exact byte cap is accepted, including an encoded native manifest.
                raw = data.encode()
                if manifest == "plugin.json":
                    raw = b"\xff\xfe" + data.encode("utf-16-le")
                    raw += b" \x00" * ((TEXT_LIMIT - len(raw)) // 2)
                else:
                    raw += b" " * (TEXT_LIMIT - len(raw))
                path = self.write(manifest, raw)
                self.assertEqual(self.invoke("identity"), ["candidate"])
                self.assertTrue(any(p["name"] == "candidate" for p in self.invoke("discovery")["plugins"]))
                with path.open("ab") as stream:
                    stream.write(b" ")
                self.refused()

    @unittest.skipUnless(os.name == "posix", "Unix special files")
    def test_fifo_and_symlink_compatibility_manifests(self):
        self.native()
        for manifest in MANIFESTS[1:]:
            for kind in ("fifo", "symlink"):
                with self.subTest(manifest=manifest, kind=kind):
                    path = self.folder / manifest
                    path.parent.mkdir(parents=True, exist_ok=True)
                    if kind == "fifo":
                        os.mkfifo(path)
                    else:
                        target = self.write("target.json", '{"name":"candidate"}')
                        path.symlink_to(target)
                    self.refused()
                    path.unlink()

    def test_tree_file_count_depth_and_total_bytes(self):
        self.native()
        for kind in ("files", "depth", "bytes", "directories"):
            with self.subTest(budget=kind), tempfile.TemporaryDirectory(dir=self.folder) as tmp:
                root = Path(tmp)
                if kind == "files":
                    for i in range(4097):
                        (root / str(i)).touch()
                elif kind == "directories":
                    for i in range(8193):
                        (root / str(i)).mkdir()
                elif kind == "depth":
                    root.joinpath(*(["deep"] * 34)).mkdir(parents=True)
                else:
                    for i in range(TREE_LIMIT // MIB + 1):
                        with (root / str(i)).open("wb") as stream:
                            stream.truncate(MIB)
                self.refused()

    def test_live_project_global_builtin_and_off_folders_are_checked(self):
        (self.home / "trusted_folders.toml").write_text(
            f"trusted = [{json.dumps(str(self.cwd))}]\nuntrusted = []\n"
        )
        for parent in (self.home / "plugins", self.cwd / ".vibe/plugins", self.home / "plugins-off",
                       self.cwd / ".vibe/plugins-off", self.base / "builtins"):
            with self.subTest(parent=parent):
                shutil.rmtree(self.folder)
                self.folder = parent / "candidate"
                self.folder.mkdir(parents=True)
                self.native()
                self.write(".claude-plugin/plugin.json", " " * (TEXT_LIMIT + 1))
                # Built-ins are normally packaged; point the explicit additional root at this fixture. The
                # folder in home/plugins counts as one the sweep kept (only those are read there).
                result = self.invoke("discovery", extra=(
                    "import sys, json\nsys.argv = [sys.argv[0], *sys.argv[1:3], "
                    f"json.dumps({{'builtinRoots': [{str(self.base / 'builtins')!r}], 'kept': ['candidate']}})]\n"
                ))
                row = next((p for p in result["plugins"] if p["path"] == str(self.folder)), None)
                self.assertIsNotNone(row, result)
                self.assertTrue(any("not readable" in s for s in row["issues"]), row)
                (self.folder / ".claude-plugin/plugin.json").unlink()
                (self.folder / ".claude-plugin").rmdir()
                (self.folder / "plugin.json").unlink()

    def test_listing_and_identity_do_not_write_generated_skills_or_migrate_config(self):
        self.write(".claude-plugin/plugin.json", '{"name":"candidate","commands":["./commands"]}')
        self.write("commands/check.md", "---\ndescription: Check changes\n---\nCheck the changes.")
        config = self.home / "config.toml"
        config.write_text('[tools.bash]\nallowlist = ["ls *"]\n')
        before = {p: p.read_bytes() for p in self.base.rglob("*") if p.is_file()}
        with self.subTest(helper="identity"):
            self.assertEqual(self.invoke("identity", audit=True), ["candidate"])
        with self.subTest(helper="discovery"):
            result = self.invoke("discovery", audit=True)
            row = next(p for p in result["plugins"] if p["name"] == "candidate")
            self.assertTrue(any(c["name"] == "candidate:command-check" for c in row["components"]), row)
        after = {p: p.read_bytes() for p in self.base.rglob("*") if p.is_file()}
        self.assertEqual(before, after)
        self.assertFalse((self.home / "plugin-data").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
