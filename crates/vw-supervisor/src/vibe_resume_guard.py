"""Keep recovered root work and plugin changes behind the app's verified resume boundaries.

Installed by the app launcher only. Stop old work before resume answers; leave saved
plugin pins for the core's verified refresh before it admits new work.
"""


def install_restored_turn_guard():
    import importlib.metadata
    if importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    from mistralai_vibe_local_harness.vibe._session import UnifiedHarnessSessionBackend

    async def interrupt_restored(self, runtime, recovered, turn_id, once_idle):
        del recovered

        async def after():
            if once_idle is not None:
                await once_idle()

        self._after_recovery = after
        # Native cancellation, idle configuration adoption and integration settlement stay together.
        await self._interrupt_core_turn(runtime, turn_id, reason="client interrupt")
        self._advance_watermark(runtime)

    UnifiedHarnessSessionBackend._continue_recovered_turn = interrupt_restored


def install_resumed_plugin_guard():
    import importlib.metadata
    if importlib.metadata.version("mistral-vibe") != "3.1.2":
        return
    from vibe.app_server._unified_harness_backend_adapter import UnifiedHarnessBackendAdapter

    async def keep_saved_plugins(self):
        # Native resume/continue otherwise re-pin best-effort before answering. That hides
        # removed pins from the core's verified refresh and silently admits added roots.
        # Keep the restored catalogue until the core checks it under the chat lock and,
        # when needed, sends plugin/reload, verifies retirement and reopens the tools.
        # Explicit reload and deferred reload at a live turn boundary remain native.
        del self

    UnifiedHarnessBackendAdapter.reload_switched_plugins = keep_saved_plugins
