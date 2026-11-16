"""Keep Vibe's remote caches (connector catalog, model catalog, organisation config, experiments) from freezing the
app server when a refresh is queued behind a fetch that has just finished.

Vibe's RemoteCache._after_flight waits with `while state.flight is not None: await shield(state.flight)`. Only the
fetch's done callback clears `state.flight`, and that callback runs from the event loop's ready queue. When the queued
refresh looks before it ran, `state.flight` is a finished task; awaiting a finished task returns at once without
yielding, so the loop spins for good and nothing else in the server ever runs again. The guard yields once in that
case, so the done callback runs and the wait ends; everything else stays Vibe's own code path.

Installed by the launcher; a Vibe outside the guarded list stops the start (vibe_guard_versions.py).
"""


def install_remote_cache_guard():
    import asyncio
    import contextlib

    require_guarded_vibe("remote cache refresh guard")  # noqa: F821 - launch.rs prepends vibe_guard_versions.py
    from vibe.core.utils.remote_cache import RemoteCache

    async def after_flight(self, key, state):
        # A check() during the wait can start another background one.
        while state.flight is not None:
            flight = state.flight
            with contextlib.suppress(Exception):
                await asyncio.shield(flight)
            if state.flight is flight:
                # Finished, but its done callback (the only thing that clears state.flight) has not run yet.
                await asyncio.sleep(0)
        state.queued = None
        return await self._start(key, state, force=True, backoff=state.queued_backoff)

    RemoteCache._after_flight = after_flight
