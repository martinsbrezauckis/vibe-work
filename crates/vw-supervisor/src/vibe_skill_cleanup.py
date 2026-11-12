"""Keep registry skill bodies when a manifest cannot establish which versions are pinned.

Loaded by the server launcher. Uses Vibe's native worker, registry availability and eviction rules;
only the app-server cleanup boundary reads pins strictly, before work and again before deletion.
"""


def install_skill_cleanup_guard():
    import asyncio
    from functools import wraps
    import importlib.metadata
    from pathlib import Path
    import tomllib

    if importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    from vibe.app_server import _skill_store_cleanup as cleanup
    from vibe.core.skills.registry import _known_manifests, _manifest, _resolved, _service, _store
    from vibe.observability.logging import logger

    original = cleanup.SkillStoreCleanup.run
    if getattr(original, "_vw_manifest_guard", False):
        return

    def pinned_versions(roots):
        # Read the record without its is_file filter: an inaccessible parent must
        # not silently make a remembered project's manifest disappear.
        known = [Path(value) for value in _known_manifests._read()]
        paths = [_manifest.global_manifest_path(), *_manifest.project_manifest_paths_sync(roots), *known]
        pinned = set(_resolved.recorded_versions())
        for path in dict.fromkeys(paths):
            try:
                with path.open("rb") as source:
                    manifest = _manifest.SkillManifest.model_validate(tomllib.load(source))
            except FileNotFoundError:
                continue  # A removed manifest no longer pins a version.
            for entry in manifest.skills:
                if not _store.is_safe_skill_id(entry.skill_id):
                    raise ValueError("invalid registry skill identity")
                version = _service._resolved_version(entry)
                if version is not None:
                    pinned.add((entry.skill_id, version))
        return pinned

    @wraps(original)
    async def guarded(self, config, roots, *, credentials):
        try:
            await asyncio.to_thread(pinned_versions, roots)
            unused = await self._worker.run("skill_store.candidates", {"roots": [str(root) for root in roots]})
            if not unused:
                return []
            served = await cleanup.still_served(config, unused, credentials=credentials)
            pinned = await asyncio.to_thread(pinned_versions, roots)
        except (OSError, ValueError):
            logger.warning("Keeping cached skills because a skills manifest could not be read.", exc_info=True)
            return []
        served = {skill_id: [version for version in versions if (skill_id, version) not in pinned]
                  for skill_id, versions in served.items()}
        if not any(served.values()):
            return []
        evicted = await self._worker.run("skill_store.evict", {"versions": served})
        return [(skill_id, version) for skill_id, version in evicted]

    guarded._vw_manifest_guard = True
    cleanup.SkillStoreCleanup.run = guarded
