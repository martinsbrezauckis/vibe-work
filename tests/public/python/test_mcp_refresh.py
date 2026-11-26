"""Real pinned MCP transports: forced refresh must never start disabled stdio.

Run with Vibe's Python; the supervisor crate invokes this regression as well.
Two runtimes use a PID-writing stdio server so a short-lived start is observable.
"""
import asyncio
from dataclasses import replace
from pathlib import Path
import sys
import tempfile
import unittest

from mistralai_vibe_local_harness.vibe._mcp_models import (
    MCPAuthorizationRef, MCPAuthorizationSnapshot, MCPDescriptorCachePolicy,
    ResolvedMCPCatalog, ResolvedMCPServerConfig,
)
from mistralai_vibe_local_harness.vibe._mcp_runtime import MCPRuntime

REPO = Path(__file__).resolve().parents[3]
SOURCE = REPO / "crates/vw-supervisor/src"
# On the old source the test exercises native forced discovery and fails on its PID marker.
guard = SOURCE / "vibe_mcp_refresh.py"
if guard.exists():
    namespace = {}
    exec((SOURCE / "vibe_guard_versions.py").read_text(), namespace)
    exec(guard.read_text(), namespace)
    namespace["install_mcp_refresh_guard"]()

FIXTURE = '''import json, os, sys
with open(sys.argv[1], "a") as marker:
    marker.write(str(os.getpid()) + "\\n")
for raw in sys.stdin:
    message = json.loads(raw)
    if "id" not in message:
        continue
    if message.get("method") == "initialize":
        result = {"protocolVersion": message["params"]["protocolVersion"],
                  "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}
    elif message.get("method") == "tools/list":
        with open(sys.argv[2]) as tool_name:
            name = tool_name.read().strip()
        result = {"tools": [{"name": name, "inputSchema": {"type": "object"}}]}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}), flush=True)
'''


class NoCredentials:
    async def resolve(self, reference):
        return MCPAuthorizationSnapshot({}, "local", reference.descriptor_revision)


class Refresh(unittest.IsolatedAsyncioTestCase):
    async def test_two_chats_refresh_disabled_and_enabled_stdio(self):
        with tempfile.TemporaryDirectory(prefix="vw-mcp-refresh-") as folder:
            root = Path(folder)
            fixture = root / "server.py"
            fixture.write_text(FIXTURE)
            paused_marker = root / "paused-pids"
            active_marker = root / "active-pids"
            tool_name = root / "tool-name"
            tool_name.write_text("ping")
            paused = ResolvedMCPServerConfig(
                name="paused", transport="stdio", command=sys.executable,
                args=(str(fixture), str(paused_marker), str(tool_name)), disabled=True,
                authorization=MCPAuthorizationRef("paused", "paused", "none", "local"),
            )
            active = replace(paused, name="active", disabled=False,
                             args=(str(fixture), str(active_marker), str(tool_name)),
                             authorization=MCPAuthorizationRef("active", "active", "none", "local"))
            catalog = ResolvedMCPCatalog("test", (paused, active))
            runtimes = [MCPRuntime(cache_root=root / str(i),
                                   cache_policy=MCPDescriptorCachePolicy(),
                                   authorization_provider=NoCredentials()) for i in range(2)]
            try:
                for name in ["ping", "ping_updated"]:
                    tool_name.write_text(name)
                    snapshots = await asyncio.gather(*(runtime.reconfigure(
                        catalog, force_remote_discovery=True) for runtime in runtimes))
                    self.assertFalse(paused_marker.exists(),
                                     "forced refresh started the disabled stdio process")
                    self.assertTrue(active_marker.exists(), "enabled stdio was not discovered")
                    for snapshot in snapshots:
                        self.assertEqual({s.name: s.status for s in snapshot.sources},
                                         {"paused": "disabled", "active": "enabled"})
                        self.assertTrue(any(route.descriptor.server_name == "active"
                                            for route in snapshot.routes.values()))
                        self.assertEqual({route.descriptor.remote_name
                                          for route in snapshot.routes.values()}, {name})
                        self.assertFalse(any(route.descriptor.server_name == "paused"
                                             for route in snapshot.routes.values()))
                self.assertTrue(paused.disabled)
                print("FIX19_NATIVE chats=2 repeated_refresh=ok paused_starts=0 enabled_tools=updated")
            finally:
                await asyncio.gather(*(runtime.aclose() for runtime in runtimes))

    async def test_cached_then_disabled_source_stays_visible_without_restart(self):
        with tempfile.TemporaryDirectory(prefix="vw-mcp-cached-") as folder:
            root = Path(folder)
            fixture = root / "server.py"
            fixture.write_text(FIXTURE)
            marker = root / "pids"
            tool_name = root / "tool-name"
            tool_name.write_text("ping")
            server = ResolvedMCPServerConfig(
                name="cached", transport="stdio", command=sys.executable,
                args=(str(fixture), str(marker), str(tool_name)),
                authorization=MCPAuthorizationRef("cached", "cached", "none", "local"),
            )
            runtime = MCPRuntime(cache_root=root / "cache", cache_policy=MCPDescriptorCachePolicy(),
                                 authorization_provider=NoCredentials())
            try:
                await runtime.reconfigure(ResolvedMCPCatalog("on", (server,)),
                                          force_remote_discovery=True)
                starts = marker.read_text()
                disabled = ResolvedMCPCatalog("off", (replace(server, disabled=True),))
                await runtime.reconfigure(disabled, force_remote_discovery=False)
                snapshot = await runtime.reconfigure(disabled, force_remote_discovery=True)
                self.assertEqual(marker.read_text(), starts, "disabled cached server restarted")
                self.assertEqual(snapshot.sources[0].status, "disabled")
                self.assertEqual(snapshot.sources[0].descriptors[0].remote_name, "ping")
                self.assertFalse(snapshot.routes)
                snapshot = await runtime.reconfigure(ResolvedMCPCatalog("empty", ()),
                                                     force_remote_discovery=True)
                self.assertFalse(snapshot.sources)
                self.assertEqual(marker.read_text(), starts)
                print("FIX19_NATIVE cached_disabled=visible paused_restarts=0 empty_catalog=ok")
            finally:
                await runtime.aclose()


if __name__ == "__main__":
    unittest.main()
