---
name: processes
description: Vibe Work's process tools - processes_list, process_start, process_stop, process_restart and port_lease for servers, watchers and long-running commands, and busy ports.
---

Vibe Work runs session processes through `mcp_vw` in `run_typescript`; one program has 60 s for all of its tool calls:

```ts
await tools.mcp_vw.processes_list({});
```

```ts
await tools.mcp_vw.process_start({ command: "python3 -m http.server $PORT --bind 127.0.0.1" }); // bash
// Windows PowerShell: "python -m http.server $PORT --bind 127.0.0.1"
```

1. `processes_list` lists this session's processes with their pids, ports and URLs, including servers started earlier that are still running ("started earlier, still running").
   The status bar's running-process count opens Processes. Left running lists agent processes that kept running after their chat was deleted, with its pid, chat title and recorded command when known. Stop ends the process and its children.
2. `process_start` (`command`, optional session `cwd` and `port`) starts a process that survives Vibe stopping and Quit Vibe Work; background shell commands and `tools.process.start` do not.
3. `PORT` is leased: `$PORT` in bash and Windows PowerShell (`$env:PORT` also works there). `port_lease` lists the leased ports. Windows runs `pwsh` 7 when it is installed (`&&` works), else 5.1 (no `&&`). The process record's `shell` says which ran.
4. `process_stop({ pid })` and `process_restart({ pid })` take a pid from `processes_list`. The list keeps the last 1 KiB of captured output for eight recent finished processes in this session (64 across the app), including after stop; older entries are evicted, and closing the app clears finished snapshots. Literal kills of other sessions/apps or nonliteral targets ask the user; literal targets of this session's own processes pass.
5. A start or restart can answer pending or unknown; `processes_list` then shows whether it runs. `preview_open` shows a listening server in the Browser tab (`vw:preview`).
6. Shell commands have a length limit; the file tools write long scripts, HTML and CSS without one.
