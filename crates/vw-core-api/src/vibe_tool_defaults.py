"""Print each Vibe tool's default allowlist as JSON, read through pinned Vibe's own tool catalogue.

Called by vibe_rules.rs in Vibe's isolated Python. Read-only: it only imports Vibe's builtin tools and instantiates
their default configs (no config file is read or written, no server starts).

Several classes can share one tool name (`Bash` and `ExperimentalBash` both use "bash" in pinned Vibe).
Vibe's `discover_tool_defaults` keeps the last class it encounters, without checking availability. This display
helper probes `is_available` without a server config and unions the defaults of available variants; if none report
available, it uses every variant as a fallback. Availability can differ from the server's configured shell rollout.
These defaults distinguish shipped allowlists from earlier "Always allow" answers; they do not grant permission.
"""
import inspect
import json

from vibe.core.tools.manager import builtin_tool_classes


def available(cls):
    try:
        if inspect.signature(cls.is_available).parameters:
            return bool(cls.is_available(None))
        return bool(cls.is_available())
    except Exception:
        return False


variants = {}
for cls in builtin_tool_classes():
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
