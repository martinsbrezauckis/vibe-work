"""Keep filtered MCP schemas out of Vibe 3.1.2's inventory-only projection.

The launcher installs this after native routing has refused invalid schemas.
It preserves native routes/warnings and never changes the accepted snapshot.
"""


def install_mcp_projection_guard(adapter):
    import importlib.metadata
    from dataclasses import replace
    from functools import wraps

    if importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    original = adapter._session_mcp_state
    if getattr(original, "_vw_schema_projection_guard", False):
        return
    from mistralai_vibe_local_harness.vibe._tool_schemas import type_is_not_object

    @wraps(original)
    def projected(snapshot, orchestrator, plugin_mcp):
        # Native groups omit these tools, but projection indexes each raw source
        # descriptor in that smaller map. Restrict only this display input;
        # the accepted descriptors still drive warning deduplication and refresh.
        sources = tuple(replace(source, descriptors=tuple(
            descriptor for descriptor in source.descriptors
            if not type_is_not_object(descriptor.input_schema))) for source in snapshot.sources)
        return original(replace(snapshot, sources=sources), orchestrator, plugin_mcp)

    projected._vw_schema_projection_guard = True
    adapter._session_mcp_state = projected
