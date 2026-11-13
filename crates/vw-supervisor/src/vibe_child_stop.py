"""Vibe Work's audited Stop, process-start, configuration and plugin-hook guards, loaded by the server launcher.

A Stop of an owned child session goes through Vibe's subagent controller and its
durable interrupt operation. Access-mode switches wait for configuration reads
and reloads before rebuilding the profile. Revisit these guards at each Vibe pin
bump. Native plugin hook subprocesses are refused before binding: they bypass
our tool guards. No installed Vibe files are changed; original handlers apply allowed operations.
An admitted background process finishes its acknowledgment even when its turn is interrupted,
so it stays listed and its own Stop can finish. Turn Stop never waits for the process to exit.
"""
import importlib.metadata
import sys


def install_config_switch_guard(adapter):
    """Wait for config readers before profile rebuilds or copying their layer locks.

    Async profile switching avoids the older deadlock, but a contended layer lock
    still retains its event loop, which makes Vibe's subsequent config copy fail.
    """
    from functools import wraps

    async def quiescent_config(orchestrator):
        import asyncio

        while True:
            # Reload takes mutation, builder and layer locks, but previews and
            # direct reads skip the first, so every one of them is checked.
            locks = [orchestrator._mutation_lock, orchestrator._builder._lock,
                     *(layer._lock for layer in orchestrator.layers)]
            # release() can leave a woken waiter with locked() == False. The worker
            # must take only Lock.acquire's uncontended path, which ignores loop
            # affinity. A non-cancelled waiter (even done) prevents that path.
            if all(
                not lock.locked() and not any(not waiter.cancelled() for waiter in lock._waiters or ())
                for lock in locks
            ):
                return
            # Poll instead of queueing: a lock that ever had a waiter stays bound
            # to this loop, and Vibe deep-copies layers together with their locks.
            await asyncio.sleep(0.01)

    async def quiescent(context):
        await quiescent_config(context.config_orchestrator)

    async def current_context(self):
        while True:
            context = self._context
            await quiescent(context)
            if context is self._context:
                return

    def guard_current(original, *, after=False):
        @wraps(original)
        async def guarded(self, *args, **kwargs):
            if after:
                result = await original(self, *args, **kwargs)
                await current_context(self)
                return result
            await current_context(self)
            # Older runtimes rebuild synchronously. Async rebuilds also need this
            # wait: queueing on a layer lock makes the later config copy fail.
            return await original(self, *args, **kwargs)
        return guarded

    backend = adapter.UnifiedHarnessBackendAdapter
    backend.switch_agent = guard_current(backend.switch_agent)
    backend._restore_profile = guard_current(backend._restore_profile)
    # Agent install/uninstall switches after this await, so guard the completion.
    # Its other callers also enter with no configuration lock held.
    backend._await_deferred_setup = guard_current(backend._await_deferred_setup, after=True)

    # Org policy first copies the layers, then switches the shared mode after a
    # yielding reload. Entry-only guarding misses both the deferred apply-lock
    # wait and a reader that starts while that reload builds another layer.
    # Keep upstream commit, refusal, rollback and cancellation semantics intact.
    from contextvars import ContextVar
    from vibe.app_server._deferred_config import DeferredConfiguration
    from vibe.core.config.orchestrator import ConfigOrchestrator

    copy_context = ContextVar("vw_config_copy_context", default=None)
    original_org = backend._commit_org_config
    original_commit = DeferredConfiguration.commit
    original_reload = ConfigOrchestrator.reload_layer

    def copying_context(original):
        @wraps(original)
        async def guarded(self, *args, **kwargs):
            await current_context(self)
            token = copy_context.set(self._context)
            try:
                return await original(self, *args, **kwargs)
            finally:
                copy_context.reset(token)
        return guarded

    @wraps(original_commit)
    async def commit(self, build, **kwargs):
        context = copy_context.get()
        if context is None:
            return await original_commit(self, build, **kwargs)

        async def guarded_build():
            # Called under DeferredConfiguration's apply lock, immediately before copy().
            await quiescent(context)
            return await build()

        return await original_commit(self, guarded_build, **kwargs)

    @wraps(original_reload)
    async def reload_layer(self, *args, **kwargs):
        result = await original_reload(self, *args, **kwargs)
        if copy_context.get() is not None:
            # Original reload released its locks. Do not queue on them, or take
            # them across a profile rebuild, which acquires them again.
            await quiescent_config(self)
        return result

    backend._commit_org_config = copying_context(original_org)
    # Catalog commits use the same deferred builder/copy boundary as org policy.
    if hasattr(backend, "_commit_model_catalog"):
        backend._commit_model_catalog = copying_context(backend._commit_model_catalog)

    # New/rescanned and saved-session plugin switches copy the same live layers.
    from vibe.app_server import _plugins
    if hasattr(_plugins, "_with_current_files"):
        original_files = _plugins._with_current_files

        @wraps(original_files)
        async def current_files(orchestrator):
            if orchestrator is not None:
                await quiescent_config(orchestrator)
            return await original_files(orchestrator)

        _plugins._with_current_files = current_files
    DeferredConfiguration.commit = commit
    ConfigOrchestrator.reload_layer = reload_layer

    original_adopt = backend.adopt_context

    @wraps(original_adopt)
    async def adopt_context(self, context):
        await quiescent(context)
        return await original_adopt(self, context)

    backend.adopt_context = adopt_context
    host = adapter.UnifiedHarnessBackendHostAdapter
    original_pin = host._apply_restored_pin

    @wraps(original_pin)
    async def apply_restored_pin(self, pin, context, value):
        if pin == adapter.SessionPin.AGENT_NAME:
            await quiescent(context)
        return await original_pin(self, pin, context, value)

    host._apply_restored_pin = apply_restored_pin


def install_plugin_hook_guard():
    """Refuse executable plugin hooks before any session binds them, including saved copies."""
    from functools import wraps
    from vibe.app_server import _runtime

    original = _runtime._runnable_plugin_hooks

    @wraps(original)
    def runnable(plugins):
        hooks, issues = original(plugins)
        if hooks:
            # Hook subprocesses do not pass through our pre_tool guards. Discovery's
            # approval check is not enough: project/built-in roots, late copies and
            # stored pins all meet here during start/resume/fork/reattach.
            raise ValueError("Native plugin hooks are not supported by Vibe Work. "
                             "Remove or turn off the plugin before opening this chat.")
        return hooks, issues

    _runtime._runnable_plugin_hooks = runnable


def install_builtin_skill_filter():
    """Hide the CLI-home import workflow until the app offers its own import flow."""
    from functools import wraps
    from vibe.core.plugins._native import PluginResolver
    from vibe.core.skills.models import SkillScope

    original = PluginResolver._load_skills

    @wraps(original)
    def load_skills(self, plugin):
        skills = original(self, plugin)
        if plugin.scope == SkillScope.BUILTIN:
            skills.pop("vibe:import", None)
        return skills

    PluginResolver._load_skills = load_skills


def install_process_start_guard():
    """Cancellation must not lose a live process's publication or leave its Stop barrier locked."""
    import asyncio
    from functools import wraps
    from mistralai_vibe_local_harness.vibe._runtime import DurableSessionRuntime

    original = DurableSessionRuntime._acknowledge_process_result_applied
    original_apply = DurableSessionRuntime._apply
    original_resolve = DurableSessionRuntime._resolve_action

    async def finish(awaitable):
        pending = asyncio.create_task(awaitable)
        cancelled = False
        while not pending.done():
            try:
                await asyncio.shield(pending)
            except asyncio.CancelledError:
                # Every wait stays shielded, including a second or later Stop.
                cancelled = True
        result = pending.result()
        if cancelled:
            raise asyncio.CancelledError
        return result

    @wraps(original_resolve)
    async def resolve(self, action, **kwargs):
        if getattr(getattr(action, "call", None), "name", None) == "process.start":
            if not hasattr(self, "_vw_process_starts"):
                self._vw_process_starts = {}
            self._vw_process_starts[action.action_id] = action
        try:
            return await original_resolve(self, action, **kwargs)
        except BaseException:
            getattr(self, "_vw_process_starts", {}).pop(action.action_id, None)
            raise

    async def acknowledged(self, action, event):
        await original(self, action, event)
        getattr(self, "_vw_process_starts", {}).pop(action.action_id, None)

    @wraps(original)
    async def acknowledge(self, action, event):
        if getattr(getattr(action, "call", None), "name", None) != "process.start":
            return await original(self, action, event)
        return await finish(acknowledged(self, action, event))

    @wraps(original_apply)
    async def apply(self, event):
        # Inspection's pending actions contain only names, not the full call
        # required by acknowledgment. Keep the actual admitted action until ack.
        action = getattr(self, "_vw_process_starts", {}).get(getattr(event, "action_id", None))
        if action is None:
            return await original_apply(self, event)

        async def apply_and_acknowledge():
            # Core removes the pending action BEFORE awaiting process persistence.
            # Cancellation there makes failure settlement skip it, so the atomic
            # unit must begin before apply, and include barrier release/publication.
            transition = await original_apply(self, event)
            await acknowledged(self, action, event)
            return transition

        return await finish(apply_and_acknowledge())

    DurableSessionRuntime._resolve_action = resolve
    DurableSessionRuntime._apply = apply
    DurableSessionRuntime._acknowledge_process_result_applied = acknowledge


def install():
    # These releases share the guarded runtime boundaries. Keep the previous pin
    # usable during upgrades; a new release still needs its own compatibility audit.
    if importlib.metadata.version("mistral-vibe") not in {"3.0.4", "3.1.0", "3.1.1", "3.1.2"}:
        return
    from vibe.app_server import _unified_harness_backend_adapter as adapter
    from mistralai_vibe_local_harness.vibe._subagents._models import IdleChild, RunningChild, TurnFailedChild
    from mistralai_vibe_local_harness.vibe._subagents._controller import success_output

    install_config_switch_guard(adapter)
    install_plugin_hook_guard()
    install_builtin_skill_filter()
    install_mcp_projection_guard(adapter)  # noqa: F821 - launch.rs prepends vibe_mcp_projection.py
    install_skill_cleanup_guard()  # noqa: F821 - launch.rs prepends vibe_skill_cleanup.py
    install_restored_turn_guard()  # noqa: F821 - launch.rs prepends vibe_resume_guard.py
    install_resumed_plugin_guard()  # noqa: F821 - launch.rs prepends vibe_resume_guard.py
    install_macos_keychain_guard()  # noqa: F821 - launch.rs prepends vibe_keychain_guard.py
    install_windows_credential_guard()  # noqa: F821 - launch.rs prepends vibe_keychain_guard.py
    install_bundled_plugin_guard()  # noqa: F821 - launch.rs prepends vibe_plugin_enablement.py
    install_process_start_guard()
    install_process_tracking()  # noqa: F821 - launch.rs prepends vibe_process_tracking.py
    install_process_output()  # noqa: F821 - launch.rs prepends vibe_process_output.py
    original = adapter.UnifiedHarnessBackendAdapter.interrupt_turn

    async def interrupt_turn(self, params):
        if params.session_id == self.session_id or not self.references_child(params.session_id):
            return await original(self, params)
        controller = self._host._subagent_controller(self.session_id)
        records = controller._subagents().children if controller is not None else {}
        for name, child in records.items():
            if child.child_session_id != params.session_id:
                continue
            if not isinstance(child.state, RunningChild):
                return adapter.SessionBackendResult(
                    response=adapter.TurnInterruptResponse(last_event_id=self._event_id)
                )
            # The controller retains the owned generation while a finished live turn settles.
            # The host interrupt validates that generation against the child's current or last turn.
            if child.state.active.turn_id != params.expected_turn_id:
                raise adapter.SessionBackendError(
                    adapter.ProtocolErrorCode.CONFLICT, "The child's active turn changed before Stop."
                )
            # Exactly the host's interrupt_running_children operation (controller.py:497-562),
            # with only this owned child's id and generation. Its receipt survives a restart.
            operation = f"host-interrupt:{child.child_session_id}:{child.state.active.generation}"
            output = await controller._interrupt_on_host_behalf(name, operation, "Stopped by the user")
            # The child can settle while the operation waits for its name/graph locks.
            settled = controller._subagents().children.get(name)
            ended = (
                settled is not None
                and settled.child_session_id == params.session_id
                and isinstance(settled.state, IdleChild | TurnFailedChild)
            )
            if output != success_output() and not ended:
                raise adapter.SessionBackendError(
                    adapter.ProtocolErrorCode.CONFLICT, "The child could not be interrupted."
                )
            return adapter.SessionBackendResult(
                response=adapter.TurnInterruptResponse(last_event_id=self._event_id)
            )
        return await original(self, params)

    adapter.UnifiedHarnessBackendAdapter.interrupt_turn = interrupt_turn


install()
sys.argv = sys.argv[1:]
# Imported after install(): the server module must load with the patched adapter in place.
from vibe.app_server.stdio import main  # noqa: E402
sys.exit(main())
