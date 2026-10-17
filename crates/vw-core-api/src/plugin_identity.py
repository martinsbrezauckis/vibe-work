"""Read one folder's identity through Vibe's resolver for the core's approval gate.

Called by plugin_identity.rs in Vibe's isolated Python environment. Only a manifest
name leaves this process; resolving components never starts MCP servers.
"""
import json
import sys
from pathlib import Path

from vibe.core.plugins import PluginResolver

root = Path(sys.argv[1]).resolve()
result = PluginResolver(plugin_dirs=[root]).resolve()
print(json.dumps([plugin.name for plugin in result.plugins if plugin.root == root]))
