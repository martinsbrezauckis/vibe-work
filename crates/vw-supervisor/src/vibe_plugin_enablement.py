"""Keep the app-owned bundled plugin enabled while respecting every other native switch.

Embedded by the launcher and read-only inventory. The trusted root comes from app launch
arguments, never a plugin name or user config. A shadow/collision is refused with recovery steps.
"""
def install_bundled_plugin_guard(roots=None, *, refuse_missing=True):
    import importlib.metadata
    if importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    from functools import wraps
    from pathlib import Path
    import sys
    from vibe.core.plugins import _enablement
    from vibe.core.plugins._native import PluginResolver
    from vibe.core.skills.models import SkillScope

    if roots is None:
        roots = [sys.argv[index + 1] for index, arg in enumerate(sys.argv[:-1])
                 if arg == "--additional-builtin-plugin-root"]
    parents = {Path(root).resolve() for root in roots if (Path(root) / "vw" / "plugin.json").is_file()}
    protected = {root / "vw" for root in parents}
    if not protected:
        return
    required = _enablement.is_required_plugin
    resolve = PluginResolver.resolve

    def is_bundled(plugin):
        return plugin.name == "vw" and plugin.scope == SkillScope.BUILTIN and plugin.root.resolve() in protected

    def is_required(plugin):
        return is_bundled(plugin) or required(plugin)

    @wraps(resolve)
    def checked(self):
        result = resolve(self)
        expected = {Path(root).resolve() / "vw" for root in self._builtin_roots if Path(root).resolve() in parents}
        if refuse_missing and expected and not any(is_bundled(plugin) for plugin in result.plugins):
            raise ValueError("Vibe Work's bundled plugin is unavailable because another plugin shadows or collides with it. "
                             "Remove or rename the conflicting plugin, then reopen the chat.")
        return result

    _enablement.is_required_plugin = is_required
    PluginResolver.resolve = checked
