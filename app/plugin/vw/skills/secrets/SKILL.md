---
name: secrets
description: Read this when a command needs an API key, token or password (a deploy, an API call, a CLI login), or when a web page shows a new key the user wants to keep.
---

The user's keys live in their vault. You never see a value and never ask the user to paste one into the chat. Use the `mcp_vw` tools inside `run_typescript`:

```ts
const { secrets } = await tools.mcp_vw.secret_list({}); // [{ id, name, fields }]: names only
```

Then, in a new call:

```ts
const result = await tools.mcp_vw.secret_run({
  program: "npm",                    // one program: a name on PATH or a path; there is no shell
  args: ["run", "deploy"],           // its arguments, one per item
  env: [{ name: "STRIPE_API_KEY", secret: "Stripe test key" }], // which secret goes into which variable
  purpose: "deploy the site to staging", // context in the tool call
});
// { exitCode, stdout, stderr } — or { status: "running", requestId } while the program still runs:
// Keep requestId. In a NEW call, use tools.mcp_vw.secret_run_status({ requestId }).
// Never loop here and never repeat secret_run for a request that has started.
```

1. The vault asks the person when needed; do not ask for permission yourself.
2. Code given inline (`sh -c`, `python -c`, `node -e`) is refused: write a script file in the project and run it (`program: "sh", args: ["deploy.sh"]`).
3. Output that contains a value comes back as `[secret withheld: vaultValue]`. Do not try to print, encode or send a value anywhere: the program gets it to do its job, nothing more.
4. A secret is not in `secret_list`: ask the user to add it in Vibe Work (Settings › Project secrets).
5. **A key on a web page:** stop and ask the user to select the key in the browser panel, press **Save to vault**, name it and confirm; do not read or copy the key. `browser_capture_secret` stays listed to explain this rule and always refuses. After the user saves it, find its name with `secret_list` and use it with `secret_run`.
6. **A key for an MCP server**: after the user saves it, tell them to add the server in Tools, pick that key, and choose "Always (until revoked)" (recommended: no prompt on every start). The MCP label only helps the user find the key; it changes no permission.
7. Vibe Work stops some commands before they run as a safety reminder (deleting your home or a drive, disk tools, shutdown, rules the user added). The reason says what to do instead; if the user really wants it, tell them the exact command so they can run it themselves.

`secret_run_status` reads the result of a long command using the request id returned by `secret_run`.
