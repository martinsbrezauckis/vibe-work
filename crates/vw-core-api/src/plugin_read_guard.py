"""Bounded, read-only inspection for the core's identity and Tools helpers.

Prepended by Rust to both isolated scripts. Depends on pinned Vibe's resolver
hooks: preflight runs before format detection, synthetic skills stay in memory,
and import-time trust initialization uses a read-only snapshot.
"""
import io
import os
import stat
import sys
import tomllib
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace

MAX_FILE_BYTES = 8 * 1024 * 1024
MAX_TREE_BYTES = 300 * 1024 * 1024
MAX_TREE_FILES = 4096
MAX_TREE_ENTRIES = 8192
MAX_TREE_DEPTH = 32
READ_ROOTS = set()
HASH_READS = set()
TEXT_SUFFIXES = {".json", ".jsonc", ".toml", ".yaml", ".yml", ".md", ".markdown",
                 ".txt", ".js", ".mjs", ".cjs", ".ts", ".tsx", ".py", ".sh"}
# Static markers in pinned _compatibility.py; OpenCode's recursive code/skill
# markers and declared components are covered by the same all-files policy.
MANIFEST_PATHS = (
    "plugin.json", ".claude-plugin/plugin.json", ".codex-plugin/plugin.json",
    "kimi.plugin.json", ".kimi-plugin/plugin.json", "package.json",
    "opencode.json", "opencode.jsonc", ".opencode/opencode.json", ".opencode/opencode.jsonc",
)


def refuse_writes(event, args):
    # This inspection process has no persistence responsibilities, even during imports.
    if event == "open" and args[2] & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC):
        raise PermissionError("Plugin inspection is read-only")
    if event == "open" and isinstance(args[0], (str, bytes)):
        path = Path(os.path.abspath(os.fsdecode(args[0])))
        if any(path.is_relative_to(root) for root in READ_ROOTS):
            # Vibe can declare text with an arbitrary suffix. Only its streaming
            # content digester may open a larger file; parsing remains bounded.
            metadata = path.lstat()
            regular(metadata)
            if metadata.st_size > MAX_FILE_BYTES and path not in HASH_READS:
                raise ValueError(f"{path.name}: parsed text exceeds 8 MiB")
    if event in {"os.mkdir", "os.rename", "os.remove", "os.rmdir", "os.symlink", "os.link",
                 "os.chmod", "os.chown", "os.truncate", "os.utime"}:
        raise PermissionError("Plugin inspection is read-only")


sys.addaudithook(refuse_writes)


def plain(metadata):
    return not (stat.S_ISLNK(metadata.st_mode) or
                getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)


def regular(metadata):
    if not plain(metadata) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError("files must be regular files without links")


def read_regular(path):
    metadata = path.lstat()
    regular(metadata)
    if metadata.st_size > MAX_FILE_BYTES:
        raise ValueError(f"{path.name}: parsed text exceeds 8 MiB")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_BINARY", 0)
    with os.fdopen(os.open(path, flags), "rb") as stream:
        regular(os.fstat(stream.fileno()))
        data = stream.read(MAX_FILE_BYTES + 1)
        regular(os.fstat(stream.fileno()))
    if len(data) > MAX_FILE_BYTES:
        raise ValueError(f"{path.name}: parsed text exceeds 8 MiB")
    return data


def check_plugin_tree(root):
    metadata = root.lstat()
    if not plain(metadata) or not stat.S_ISDIR(metadata.st_mode):
        raise ValueError("the plugin must be a directory without links")
    READ_ROOTS.add(root.resolve())
    pending = [(root, 0)]
    entries = files = size = 0
    while pending:
        directory, depth = pending.pop()
        # Stream enumeration: sorting/rglob before checking would itself be unbounded.
        with os.scandir(directory) as children:
            for child in children:
                entries += 1
                if entries > MAX_TREE_ENTRIES:
                    raise ValueError("the tree exceeds 8192 entries")
                if depth + 1 > MAX_TREE_DEPTH:
                    raise ValueError("the tree exceeds 32 levels")
                metadata = child.stat(follow_symlinks=False)
                if not plain(metadata):
                    raise ValueError("the tree contains a link")
                path = Path(child.path)
                if stat.S_ISDIR(metadata.st_mode):
                    pending.append((path, depth + 1))
                    continue
                regular(metadata)
                files += 1
                size += metadata.st_size
                if files > MAX_TREE_FILES:
                    raise ValueError("the tree exceeds 4096 files")
                if size > MAX_TREE_BYTES:
                    raise ValueError("the tree exceeds 300 MiB")
                # Bundled Node/native binaries count toward the tree bound, but are not parsed.
                # Check text handles and encoded manifests; the audit hook bounds other parser reads.
                if path.suffix.lower() in TEXT_SUFFIXES:
                    data = read_regular(path)
                    size += max(0, len(data) - metadata.st_size)
                if size > MAX_TREE_BYTES:
                    raise ValueError("the tree exceeds 300 MiB")
    for relative in MANIFEST_PATHS:
        try:
            metadata = (root / relative).lstat()
        except FileNotFoundError:
            continue
        regular(metadata)


class TrustSnapshot:
    """Vibe eagerly seeds/repairs this file at import; inspection needs only its data."""
    def __init__(self, path):
        try:
            self.data = read_regular(path)
            tomllib.loads(self.data.decode("utf-8"))
        except (OSError, ValueError, UnicodeError):
            self.data = b""

    def is_file(self):
        return True

    def open(self, mode):
        if mode != "rb":
            raise PermissionError("The trust snapshot is read-only")
        return io.BytesIO(self.data)


from vibe.core import paths  # noqa: E402 - the write audit hook must precede Vibe imports.

paths.TRUSTED_FOLDERS_FILE = SimpleNamespace(
    path=TrustSnapshot(paths.TRUSTED_FOLDERS_FILE.path)
)

from vibe.core.plugins import PluginResolver  # noqa: E402 - the read-only trust snapshot must precede resolver imports.
from vibe.core.plugins import _content  # noqa: E402 - pinned streaming package digester.
from vibe.core.plugins._native import PluginConfigIssue  # noqa: E402 - the read-only trust snapshot must precede resolver imports.

_digest_file = _content._digest_file


def hash_regular(path):
    # Vibe 3 hashes every package file in 1 MiB chunks. The preflight has already
    # bounded the whole tree; permit this read without treating a binary as text.
    path = Path(os.path.abspath(path))
    HASH_READS.add(path)
    try:
        return _digest_file(path)
    finally:
        HASH_READS.discard(path)


_content._digest_file = hash_regular


class ReadOnlyPluginResolver(PluginResolver):
    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.unreadable = {}

    def _load_plugin(self, plugin_dir, scope):
        try:
            check_plugin_tree(plugin_dir)
            return super()._load_plugin(plugin_dir, scope)
        except (OSError, ValueError) as error:
            message = f"Plugin is not readable: {error}"
            self.unreadable[str(plugin_dir.absolute())] = (scope, message)
            self._issues.append(PluginConfigIssue(file=plugin_dir, message=message, fatal=True))
            return None

    def _load_adapted_skills(self, plugin, definitions):
        # The pinned resolver writes only for synthetic_skill. Inspection needs the
        # source skill and alias, never its generated runtime path or translation tag.
        return super()._load_adapted_skills(
            plugin, tuple(replace(item, translation=None) for item in definitions)
        )

    def _child_dirs(self, roots):
        folders = []
        for root in roots:
            try:
                with os.scandir(root) as children:
                    for count, child in enumerate(children, 1):
                        if count > MAX_TREE_ENTRIES:
                            raise ValueError("a discovery root exceeds 8192 entries")
                        if child.is_dir(follow_symlinks=False) or child.is_symlink():
                            folders.append(Path(child.path))
            except FileNotFoundError:
                continue
        return sorted(folders)

    @property
    def _builtin_plugin_dirs(self):
        return [folder for folder in self._child_dirs(self._builtin_roots)
                if not folder.name.startswith("__") and (folder / "plugin.json").exists()]


def unreadable_rows(resolver, off=False):
    return [{"name": Path(path).name, "description": "Plugin is not readable.",
             "path": path, "scope": scope.value, "off": off, "available": False,
             "readable": False, "components": [], "servers": [], "issues": [message]}
            for path, (scope, message) in resolver.unreadable.items()]
