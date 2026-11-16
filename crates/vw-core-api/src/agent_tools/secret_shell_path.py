"""Read the Windows agent shell's PATH for secret_env.rs, without any vault values.

Uses pinned Vibe's terminal resolver and argv builder. Git's launcher supplies its tool
folders; cygpath converts that shell's PATH back for a native child, without inventing entries.
PowerShell does not change PATH (-NoProfile), so its inherited value is already correct.
secret_env.rs prepends vibe_guard_versions.py: Vibe's private terminal backend is imported only from a guarded Vibe.
"""
import json
import os
import subprocess

require_guarded_vibe("agent shell PATH reader")  # noqa: F821 - secret_env.rs prepends vibe_guard_versions.py
from mistralai_vibe_local_harness.vibe._processes._windows import WindowsTerminalBackend  # noqa: E402
from vibe.core.utils import get_windows_bash_path  # noqa: E402

environment = dict(os.environ)
path = next((v for k, v in environment.items() if k.upper() == "PATH"), "")
if get_windows_bash_path() is not None:
    backend = WindowsTerminalBackend("git_bash")
    shell = backend.resolve_shell(None, environment)
    result = subprocess.run(
        backend.command_argv(shell, '/usr/bin/cygpath -wp "$PATH"', interactive=False),
        env=environment,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        check=True,
        timeout=5,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    path = result.stdout.decode("utf-8").rstrip("\r\n")
print(json.dumps(path))
