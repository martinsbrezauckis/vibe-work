"""Keep disabled command servers out of the pinned Vibe's forced MCP discovery.

Installed by the launcher before any runtime exists. Native disabled-source
projection and cached descriptors stay intact; no config or installed file changes.
"""


def install_mcp_refresh_guard():
    require_guarded_vibe("disabled command MCP refresh guard")  # noqa: F821 - launcher prepends version guard
    from functools import wraps
    from mistralai_vibe_local_harness.vibe._mcp_runtime import MCPRuntime

    original = MCPRuntime._resolve_isolated
    if getattr(original, "_vw_disabled_stdio_guard", False):
        return

    @wraps(original)
    async def resolve(self, server, semaphore, *, forced, cached_only, explicit):
        # Native forced discovery ignores disabled, then starts stdio to list tools.
        # Taking its ordinary disabled branch preserves the source without contacting it.
        if server.transport == "stdio" and server.disabled:
            forced = False
        return await original(self, server, semaphore, forced=forced,
                              cached_only=cached_only, explicit=explicit)

    resolve._vw_disabled_stdio_guard = True
    MCPRuntime._resolve_isolated = resolve
