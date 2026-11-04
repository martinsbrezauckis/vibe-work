"""Register native background processes with Core before Vibe can reap their pids.

Included by the pinned launcher. A private loopback receipt carries session and process
ids and the launcher's native start token; Core verifies it in vw-procmon. Credentials never enter a
shell environment. A failed registration stops the new terminal before reporting failure.
"""


def install_process_tracking():
    import json
    import os
    import socket
    import time
    from functools import wraps
    from mistralai_vibe_local_harness.vibe._processes._backend import TerminalBackendError
    from mistralai_vibe_local_harness.vibe._processes._manager import SessionProcessManager

    endpoint = os.environ.pop("VW_PROCESS_TRACKER", None)
    if endpoint is None:
        return  # Standalone supervisor clients have no Core process monitor.
    port, token = endpoint.split(":", 1)
    original = SessionProcessManager.__init__

    def report(receipt):
        # The listener never waits for an unauthenticated peer. A connect/write
        # race, fragmented token, or lost acknowledgment requires a fresh attempt.
        # Replaying the same session/process identity is idempotent in Core.
        payload = token.encode("ascii") + b"\n" + json.dumps(receipt).encode() + b"\n"
        deadline = time.monotonic() + 1.0

        def budget():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Process registration deadline elapsed")
            return min(0.2, remaining)

        for attempt in range(32):
            try:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                with socket.create_connection(("127.0.0.1", int(port)),
                                              timeout=budget()) as conn:
                    conn.settimeout(budget())
                    conn.sendall(payload)
                    response = b""
                    while len(response) < 16 and not response.endswith(b"\n"):
                        conn.settimeout(budget())
                        part = conn.recv(16 - len(response))
                        if not part:
                            break
                        response += part
                if response in (b"ok\n", b"gone\n"):
                    return response
            except OSError:
                pass
            remaining = deadline - time.monotonic()
            if attempt < 31 and remaining > 0:
                time.sleep(min(0.01, remaining))
        raise OSError("Process registration unavailable")

    class TrackingBackend:
        def __init__(self, backend, session):
            self.backend = backend
            self.session = session
            self.request = None

        def __getattr__(self, name):
            return getattr(self.backend, name)

        def start_terminal(self, **kwargs):
            terminal = self.backend.start_terminal(**kwargs)
            try:
                request = self.request
                pid = terminal.pid
                receipt = {"session": self.session,
                           "id": request.process_id, "command": request.command,
                           "pid": pid,
                           "startToken": native_process_start_token(pid)}  # noqa: F821
                response = report(receipt)
                if response == b"gone\n" and terminal.poll() is not None:
                    # A very short command can exit before Core reads its identity.
                    # Reap any remaining tree rather than leave an untracked descendant.
                    self.backend.stop_terminals([terminal], 2.0)
                elif response != b"ok\n":
                    raise OSError("Process registration refused")
            except Exception as error:
                self.backend.stop_terminals([terminal], 2.0)
                terminal.close()
                raise TerminalBackendError("Background process could not be tracked") from error
            return terminal

    @wraps(original)
    def initialize(self, *, session_root, backend, **kwargs):
        original(self, session_root=session_root,
                 backend=TrackingBackend(backend, session_root.name), **kwargs)

    start = SessionProcessManager._start

    @wraps(start)
    def tracked_start(self, request):
        # One operation worker per manager; the backend is private to that manager.
        self._backend.request = request
        try:
            return start(self, request)
        finally:
            self._backend.request = None

    SessionProcessManager.__init__ = initialize
    SessionProcessManager._start = tracked_start
