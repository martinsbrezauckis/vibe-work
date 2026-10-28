"""Read installed plugins through Vibe's resolver for the Tools board.

Called by market_discovery after plugin_read_guard.py with the app's home and
project. Only public inventory leaves; preflight precedes detection, with no writes.
"""
import asyncio
import json
import sys
from pathlib import Path

from vibe.app_server._plugins import packaged_builtin_plugin_roots
from vibe.core.config.harness_files import HarnessFilesManager, init_harness_files_manager
from vibe.core.config import VibeConfigSchema
from vibe.core.config.layers.admin import AdminConfigLayer
from vibe.core.config.layers.agent_profile import AgentProfileLayer
from vibe.core.config.layers.default import DefaultConfigLayer
from vibe.core.config.layers.environment import EnvironmentLayer
from vibe.core.config.layers.growthbook import GrowthbookLayer
from vibe.core.config.layers.overrides import OverridesLayer
from vibe.core.config.layers.project import ProjectConfigLayer
from vibe.core.config.layers.user import UserConfigLayer
from vibe.core.config.orchestrator import ConfigOrchestrator
from vibe.core.plugins._naming import plugin_mcp_group_name
from vibe.core.skills.models import SkillScope

home, cwd = map(Path, sys.argv[1:3])
options = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
# Folders the core's sweep kept in home/plugins just before this read. Any other folder there was copied in
# after it: not read (Windows could not move a folder while its files are open) until a sweep has checked it.
swept_root = (home / "plugins").resolve()
kept = set(options.get("kept", []))


class ToolsReader(ReadOnlyPluginResolver):  # noqa: F821 - plugin_read_guard.py is prepended by Rust.
    def _child_dirs(self, roots):
        return [folder for folder in super()._child_dirs(roots)
                if folder.name in kept or folder.parent.resolve() != swept_root]


builtins = [*packaged_builtin_plugin_roots(), *(Path(p) for p in options.get("builtinRoots", []))]
workspace = [Path(p) for p in options.get("workspaceRoots", [])]
init_harness_files_manager("user", "project", additional_dirs=workspace)
files = HarnessFilesManager(sources=("user", "project")).for_session(cwd, workspace_roots=workspace)


async def inspection_config():
    # Same layer precedence as the pinned default builder, without its persistent
    # migrations or SSL initialization. The resolver only reads configured MCP names.
    overrides = OverridesLayer(data={})
    layers = [DefaultConfigLayer(schema=VibeConfigSchema), GrowthbookLayer(),
              UserConfigLayer(path=files.user_config_file)]
    if files.project_source_enabled:
        layers.append(ProjectConfigLayer(path=files.cwd, trust_store=files.trust_store))
    layers.extend([EnvironmentLayer(schema=VibeConfigSchema), overrides,
                   AgentProfileLayer(), AdminConfigLayer()])
    return await ConfigOrchestrator.create(
        schema=VibeConfigSchema, layers=layers, default_layer_resolver=lambda: overrides,
    )


config = asyncio.run(inspection_config())
resolver = ToolsReader.from_harness_files(
    files, builtin_roots=builtins,
    data_root_base=home / "plugin-data", config_orchestrator=config,
)
resolution = resolver.resolve()


def project(result, off=False):
    rows = []
    for plugin in result.plugins:
        components = []
        for name, skill in result.skills.items():
            if name.partition(":")[0] == plugin.namespace:
                components.append({"kind": "skill", "name": name, "status": None})
        for kind, collection, field in (
            ("mcp_server", result.mcp_servers, "source_id"),
            ("knowledge", result.knowledge, "name"),
            ("agent", result.agents, "name"),
            ("library", result.libraries, "alias"),
            ("connector", result.connectors, "source_id"),
            ("hook", result.runtime_hooks, "declared_name"),
        ):
            for item in collection:
                if getattr(item, "plugin_name", None) == plugin.name:
                    components.append({"kind": kind, "name": getattr(item, field), "status": None})
        issues = [issue.message for issue in result.issues if issue.file.is_relative_to(plugin.root)]
        issues.extend(item.reason for item in result.unsupported_components if item.plugin_name == plugin.name)
        rows.append({"name": plugin.name, "description": plugin.description,
                     "version": plugin.version, "author": plugin.author,
                     "path": str(plugin.root), "scope": plugin.scope.value,
                     "off": off, "available": not off, "components": components, "issues": issues,
                     "servers": [{"name": item.source_id, "group": plugin_mcp_group_name(plugin.namespace, item.source_id)}
                                 for item in result.mcp_servers if item.plugin_name == plugin.name]})
    return rows


rows = project(resolution) + unreadable_rows(resolver)  # noqa: F821 - plugin_read_guard.py is prepended by Rust.
selected = {plugin.root for plugin in resolution.plugins}
# Full resolution owns precedence. Resolve its remaining discovered folders
# independently only to describe valid plugins it shadowed or collided with.
candidates = (
    (SkillScope.PROJECT, resolver._child_dirs(files.project_plugins_dirs)),
    (SkillScope.GLOBAL, resolver._child_dirs(files.user_plugins_dirs)),
    (SkillScope.BUILTIN, resolver._builtin_plugin_dirs),
)
for scope, folders in candidates:
    for folder in folders:
        if folder.resolve() in selected or str(folder.absolute()) in resolver.unreadable:
            continue
        reader = ToolsReader(plugin_dirs=[folder], plugin_scopes={folder.resolve(): scope}, config_orchestrator=config)
        isolated = reader.resolve()
        for row in project(isolated) + unreadable_rows(reader):  # noqa: F821 - plugin_read_guard.py is prepended by Rust.
            row["available"] = False
            winner = next((p for p in resolution.plugins if p.name == row["name"]), None)
            if winner is not None:
                row["issues"].append(f"Vibe selected the plugin with this name from {winner.root}.")
            row["issues"].extend(issue.message for issue in resolution.issues if issue.file.is_relative_to(folder.resolve()))
            rows.append(row)
# Off folders are app-owned inventory, outside Vibe's discovery roots. Resolve
# each independently so a live namesake cannot hide an off plugin from the board.
for root in (home / "plugins-off", cwd / ".vibe" / "plugins-off"):
    if root.is_dir() and not root.is_symlink():
        for folder in resolver._child_dirs([root]):
            reader = ToolsReader(plugin_dirs=[folder])
            off_rows = project(reader.resolve(), True) + unreadable_rows(reader, True)  # noqa: F821 - plugin_read_guard.py is prepended by Rust.
            for row in off_rows:
                row["scope"] = "global" if root.parent == home else "project"
            rows.extend(off_rows)
print(json.dumps({"plugins": rows, "dropped": [
    {"file": str(issue.file), "message": issue.message} for issue in resolution.issues
]}))
