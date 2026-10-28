"""Read one folder's identity through Vibe's resolver for the core's approval gate.

Called by plugin_identity.rs after plugin_read_guard.py in Vibe's isolated Python
environment. Only a name leaves; preflight precedes detection and no files are written.
"""
import json
import sys
from pathlib import Path

root = Path(sys.argv[1]).absolute()
result = ReadOnlyPluginResolver(plugin_dirs=[root]).resolve()  # noqa: F821 - plugin_read_guard.py is prepended by Rust.
print(json.dumps([plugin.name for plugin in result.plugins if plugin.root == root.resolve()]))
