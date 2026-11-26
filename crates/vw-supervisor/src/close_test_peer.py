#!/usr/bin/python3
"""Disposable multiplex peer for supervisor interrupt/kill tests (no Vibe installation)."""

import argparse
import json
import subprocess
import sys
from pathlib import Path


def send(channel, message):
    sys.stdout.write(json.dumps({"channelId": channel, "message": message}) + "\n")
    sys.stdout.flush()


def reply(channel, request, result):
    send(channel, {"jsonrpc": "2.0", "id": request["id"], "result": result})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--marker", required=True)
    parser.add_argument("--mode", choices=("ack", "ignore"), required=True)
    args, _ = parser.parse_known_args()
    marker = Path(args.marker)
    running = False
    tool = None
    for line in sys.stdin:
        frame = json.loads(line)
        channel = frame["channelId"]
        request = frame["message"]
        if request is None:
            send(channel, None)
            continue
        method = request.get("method")
        if method == "initialize":
            reply(channel, request, {"serverInfo": {"name": "close-test", "version": "3.1.5"}})
        elif method == "session/start":
            tool = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    "import pathlib,sys,time; time.sleep(1.5); pathlib.Path(sys.argv[1]).write_text('done')",
                    str(marker.with_suffix(".done")),
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            marker.with_suffix(".started").write_text("started")
            running = True
            reply(channel, request, {"state": {"session": {"id": "chat-a", "status": {"type": "running", "activeTurnId": "turn-1"}}}})
        elif method == "session/read":
            status = {"type": "running", "activeTurnId": "turn-1"} if running else {"type": "idle"}
            reply(channel, request, {"state": {"session": {"id": "chat-a", "status": status}}})
        elif method == "turn/interrupt":
            if args.mode == "ack":
                marker.with_suffix(".interrupt").write_text("interrupt")
                if tool is not None:
                    tool.terminate()
                    tool.wait(timeout=1)
                running = False
                reply(channel, request, {"accepted": True})
            # Deliberately never answer in ignore mode. The tool child keeps running.
        elif "id" in request:
            reply(channel, request, {})


if __name__ == "__main__":
    main()
