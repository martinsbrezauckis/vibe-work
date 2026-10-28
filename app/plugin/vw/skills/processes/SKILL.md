---
name: processes
description: Read this before starting, reusing, stopping or restarting a server, watcher or long-running command in Vibe Work, or when a port is busy. It gives the exact tool calls.
---

Vibe Work runs session processes through `mcp_vw` in `run_typescript`:

Run each example tool call in a separate program; carry returned values into the next call.

```ts
await tools.mcp_vw.processes_list({});
await tools.mcp_vw.process_start({ command: "python3 -m http.server $PORT --bind 127.0.0.1" }); // bash
// Windows PowerShell: "python -m http.server $PORT --bind 127.0.0.1"
```

1. Call `processes_list` first. Reuse this folder's running server, including "started earlier, still running", and give its URL. Stop or replace it only when the user asks.
2. Use `process_start` (`command`, optional session `cwd` and `port`). It survives Vibe and the app closing; background shell commands and `tools.process.start` do not.
3. `PORT` is leased: use `$PORT` in bash and Windows PowerShell (`$env:PORT` also works there). `port_lease` lists ports; never guess. Windows prefers `pwsh` 7 (`&&` works), else 5.1 (use `;` with an exit check). The process record's `shell` says which ran.
4. Use `process_stop({ pid })` / `process_restart({ pid })`, taking pid from `processes_list`. The list keeps the last 1 KiB of captured output for eight recent finished processes in this session (64 across the app), including after stop; older entries are evicted, and closing the app clears finished snapshots. Literal kills of other sessions/apps or nonliteral targets ask the user; your own literal targets pass.
5. Pending/unknown start or restart: check `processes_list` before repeating. Give the listening URL and show it with `preview_open` (`vw:preview`). Fix output errors before restarting.
6. Write long scripts, HTML and CSS with the file tools, then run a short command using the file path. Avoid embedding large content in shell commands.
