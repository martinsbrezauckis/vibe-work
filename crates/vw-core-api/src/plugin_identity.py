"""Read one folder's identity through Vibe's resolver for the core's approval gate.

Called by plugin_identity.rs after plugin_read_guard.py in Vibe's isolated Python
environment. Only a name leaves; preflight precedes detection and no files are written.
"""
import json
import sys
from pathlib import Path

def names(folder):
    root = Path(folder).absolute()
    result = ReadOnlyPluginResolver(plugin_dirs=[root]).resolve()  # noqa: F821 - plugin_read_guard.py is prepended by Rust.
    return [plugin.name for plugin in result.plugins if plugin.root == root.resolve()]


# Independent resolutions keep duplicate names in different folders from shadowing each other.
if sys.argv[1] == "--batch":
    print(json.dumps([names(folder) for folder in sys.argv[2:]]))
else:
    print(json.dumps(names(sys.argv[1])))
