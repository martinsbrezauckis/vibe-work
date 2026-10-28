"""Vibe Work's 3.0.2 child Stop and configuration-switch guards, loaded by the server launcher.

A Stop of an owned child session goes through Vibe's subagent controller and its
durable interrupt operation. Access-mode switches wait for configuration reads
and reloads before rebuilding synchronously. Revisit both at each Vibe pin
bump. No installed Vibe files are changed; the original handlers still apply each operation.
"""
import importlib.metadata
import sys


def install_config_switch_guard(adapter):
    """Keep synchronous profile rebuilds clear of tasks that need the server loop."""
    from functools import wraps

    async def quiescent(context):
        import asyncio

        orchestrator = context.config_orchestrator
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
            # No yielding await precedes switch_profile in these pinned handlers.
            return await original(self, *args, **kwargs)
        return guarded

    backend = adapter.UnifiedHarnessBackendAdapter
    backend.switch_agent = guard_current(backend.switch_agent)
    backend._restore_profile = guard_current(backend._restore_profile)
    # Agent install/uninstall switches after this await, so guard the completion.
    # Its other callers also enter with no configuration lock held.
    backend._await_deferred_setup = guard_current(backend._await_deferred_setup, after=True)

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


def install():
    if importlib.metadata.version("mistral-vibe") != "3.0.2":
        return
    from vibe.app_server import _unified_harness_backend_adapter as adapter
    from mistralai_vibe_local_harness.vibe._subagents._models import IdleChild, RunningChild, TurnFailedChild
    from mistralai_vibe_local_harness.vibe._subagents._controller import success_output

    install_config_switch_guard(adapter)
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
            output = await controller._interrupt_on_host_behalf(name, operation, "Stopped by the person")
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
