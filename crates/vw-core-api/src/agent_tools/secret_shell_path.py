"""Read the Windows agent shell's PATH for secret_env.rs, without any vault values.

Uses pinned Vibe's terminal resolver and argv builder. Git's launcher supplies its tool
folders; cygpath converts that shell's PATH back for a native child, without inventing entries.
PowerShell does not change PATH (-NoProfile), so its inherited value is already correct.
"""
import json
import os
import subprocess

from mistralai_vibe_local_harness.vibe._processes._windows import WindowsTerminalBackend
from vibe.core.utils import get_windows_bash_path

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
