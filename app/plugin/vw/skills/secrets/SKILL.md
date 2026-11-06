---
name: secrets
description: Vibe Work's vault tools - secret_list and secret_run give a command an API key, token or password (a deploy, an API call, a CLI login) without showing its value; keys a web page shows.
---

Vault values never reach the agent. `mcp_vw` tools run inside `run_typescript`:

```ts
const { secrets } = await tools.mcp_vw.secret_list({}); // [{ id, name, fields }]: names only
```

`secret_run` requires at least one secret mapping in `env`; an empty or omitted mapping is refused.
Scripts without secrets run with shell or process tools.

```ts
const result = await tools.mcp_vw.secret_run({
  program: "npm",                    // program on PATH or a path; no shell
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
5. **Page keys:** the user selects the key in the browser panel, chooses **Save to vault** in the toolbar, in **Browser options** when narrow, or by right-click on the selection, then names it and confirms. `browser_capture_secret` always refuses. It appears in `secret_list` for `secret_run`.
6. **A key for an MCP server:** the user adds the server in Tools and picks the saved key; "Always allow" starts it without a prompt each time. The MCP label only helps the user find the key; it changes no permission.
7. Vibe Work refuses some commands before they run as a safety reminder (deleting your home or a drive, disk tools, shutdown, rules the user added); the refusal names the reason. The user can run such a command in their own terminal.
8. Settings > Network keeps proxy sign-in in the vault, hidden from the agent. Agent commands receive proxy addresses without the sign-in.
