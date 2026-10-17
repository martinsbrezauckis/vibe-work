"""Start the tagged Vibe stdio server for the opt-in Rust test.

The parent supplies a disposable home and clean environment. Python's audit
hook rejects Internet sockets before Vibe imports; no model turn is submitted.
"""

import io
import runpy
import socket
import subprocess
import sys
import tarfile
import tempfile


def deny_network(event, args):
    if event == "socket.__new__" and args[1] in (socket.AF_INET, socket.AF_INET6):
        raise PermissionError("Internet sockets disabled for the skills live test")


archive = subprocess.check_output(["git", "-C", sys.argv[1], "archive", "v2.26.1"])
with tempfile.TemporaryDirectory(prefix="vibe-skills-server-") as source:
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(source, filter="data")
    sys.path.insert(0, source)
    sys.addaudithook(deny_network)
    sys.argv = ["vibe.app_server.entrypoint", "--legacy-harness"]
    runpy.run_module("vibe.app_server.entrypoint", run_name="__main__")
