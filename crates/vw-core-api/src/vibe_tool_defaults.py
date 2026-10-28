"""Print each Vibe tool's default allowlist as JSON, read through pinned Vibe's own tool catalogue.

Called by vibe_rules.rs in Vibe's isolated Python. Read-only: it only imports Vibe's builtin tools and instantiates
their default configs (no config file is read or written, no server starts).

Several classes can share one tool name (Vibe 2.26.1: `Bash` and `ExperimentalBash` are both "bash") and the server
uses one that is available on this machine. Vibe's own `discover_tool_defaults` keeps whichever class it met last: on
Windows that is `ExperimentalBash`, which is never available there and lists cmd's commands (`dir`, `type`), while the
server's `Bash` with Git Bash lists the POSIX ones (`cat`, `ls`, `find`), which are Vibe's own defaults, not earlier
"Always allow" answers. So the classes a machine cannot use are skipped (`is_available`), and a name
with several usable classes gets the union of their defaults: an entry any of them ships is Vibe's own.
"""
import inspect
import json

from vibe.core.tools.manager import DEFAULT_TOOL_DIR, ToolManager


def available(cls):
    try:
        if inspect.signature(cls.is_available).parameters:
            return bool(cls.is_available(None))
        return bool(cls.is_available())
    except Exception:
        return False


variants = {}
for cls in ToolManager._iter_tool_classes([DEFAULT_TOOL_DIR.path]):
    try:
        variants.setdefault(cls.get_name(), []).append(cls)
    except Exception:
        continue

defaults = {}
for name, classes in variants.items():
    usable = [cls for cls in classes if available(cls)] or classes
    patterns = []
    for cls in usable:
        try:
            config = cls._get_tool_config_class()().model_dump(exclude_none=True)
        except Exception:
            continue
        for pattern in config.get("allowlist") or []:
            if isinstance(pattern, str) and pattern not in patterns:
                patterns.append(pattern)
    defaults[name] = patterns

print(json.dumps(defaults))
