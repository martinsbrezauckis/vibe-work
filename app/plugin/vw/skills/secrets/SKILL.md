---
name: secrets
description: Vibe Work's vault tools - secret_list and secret_run give a command an API key, token or password (a deploy, an API call, a CLI login) without showing its value; keys a web page shows.
---

The user's keys live in their vault; their values never reach the agent. The tools are on `mcp_vw` inside `run_typescript`:

```ts
const { secrets } = await tools.mcp_vw.secret_list({}); // [{ id, name, fields }]: names only
```

In a later call:

```ts
const result = await tools.mcp_vw.secret_run({
  program: "npm",                    // one program: a name on PATH or a path; there is no shell
  args: ["run", "deploy"],           // its arguments, one per item
  env: [{ name: "STRIPE_API_KEY", secret: "Stripe test key" }], // which secret goes into which variable
  purpose: "deploy the site to staging", // context in the tool call
});
// { exitCode, stdout, stderr } — or { status: "running", requestId } while the program still runs.
// tools.mcp_vw.secret_run_status({ requestId }) reads that result in a later program (one program has 60 s);
// a second secret_run starts the command again.
```

1. The vault asks the user when the secret's permission needs it.
2. Code given inline (`sh -c`, `python -c`, `node -e`) is refused; a script file in the project runs (`program: "sh", args: ["deploy.sh"]`).
3. Output that contains a value comes back as `[secret withheld: vaultValue]`.
4. The user adds secrets in Vibe Work (Settings > Project secrets); one that is missing is not in `secret_list`.
5. **A key on a web page:** saving it is the user's action: they select the key in the browser panel, press **Save to vault**, name it and confirm. `browser_capture_secret` always refuses. The saved key then appears in `secret_list` for `secret_run`.
6. **A key for an MCP server:** the user adds the server in Tools and picks the saved key; "Always allow" starts it without a prompt each time. The MCP label only helps the user find the key; it changes no permission.
7. Vibe Work refuses some commands before they run as a safety reminder (deleting your home or a drive, disk tools, shutdown, rules the user added); the refusal names the reason. The user can run such a command in their own terminal.
8. Settings > Network keeps proxy sign-in in the vault, hidden from the agent. Agent commands receive proxy addresses without the sign-in.

`secret_run_status` reads the result of a long command using the request id returned by `secret_run`.
