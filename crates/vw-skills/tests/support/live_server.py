"""Start the installed Vibe 3 server for the opt-in Rust skills tests.

The parent supplies a disposable home and clean environment. Python's audit
hook rejects Internet sockets before Vibe imports; no model turn is submitted.
"""

import importlib.metadata
import socket
import sys


def deny_network(event, args):
    if event == "socket.__new__" and args[1] in (socket.AF_INET, socket.AF_INET6):
        raise PermissionError("Internet sockets disabled for the skills live test")


assert importlib.metadata.version("mistral-vibe") == "3.0.2"
sys.addaudithook(deny_network)
# Import only after the audit hook is active.
importlib.import_module("vibe.app_server.stdio").main()
