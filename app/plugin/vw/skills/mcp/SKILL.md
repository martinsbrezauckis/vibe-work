---
name: mcp
description: Connecting any MCP server in Vibe Work - address, no sign-in, API key, sign-in, a command on this computer, certificates, failures.
---

The user adds an MCP server in Tools > Add MCP server: an address, an optional name, one of four choices. Walk them through it; you cannot add servers or see a key.

## Address
- `https://` for servers on the internet; `http://` only for one on this computer: `localhost`, `127.0.0.1` or `::1`, with its port (`http://127.0.0.1:3000/mcp`).
- No user name, password, `?query` or `#fragment` in the address; a token in it is refused: choose API key.
- Use the address from the server's documentation, usually ending in `/mcp`. A name works once: remove the server first to add it again.

## Which choice
- **No sign-in**: no key or token needed (an open server, or one the user runs here). Vibe connects at once.
- **API key**: a key or token on each request. It stays in the vault and Vibe Work adds it to each request. The default header is `Authorization: Bearer`; a server naming another (such as `X-Api-Key`) needs that name entered. Never ask for the key in chat.
- **Sign in**: a sign-in page opens in the user's browser, one at a time. Some services allow only approved apps and may refuse it.
- **Command on this computer**: a program the user names (like `npx` or `uvx` with arguments) starts the server; no address. It needs Developer permissions: until the user turns them on in Settings > Developer it is greyed with that reason. A system dialog shows the resolved program, arguments, folder and variable names; it runs with the user's account's rights, unvouched by Vibe Work. Only the user adds or changes one; never ask for a key in chat (key-like values are refused). With Developer permissions off they show Paused in Tools and resume when it is on.
- Unsure: ask what the documentation says. A wrong choice: remove the server, add it again.

## The confirmation
A system dialog shows the address. For a server on this computer it says its tools can reach what the user's account can reach, and that Vibe Work does not vouch for servers. The user decides.

## Private certificates
"Certificate not trusted" on an `https` server with a company or self-made certificate: the user adds its .pem in Settings > Network > Company certificate file, then restarts Vibe Work. A server on this computer can use `http://localhost` instead.

## What a failure means
- Certificate not trusted: see above.
- Can't reach it / No answer: not running, wrong address or port, or the network blocks it. Check, fix, refresh.
- Key rejected (401/403): wrong key or header, or no access. Remove the server and add it with another key.
- Needs credentials: added with No sign-in but wants a key or sign-in. Remove it and add it again.
- Access refused (403): the account may not use it.
- Needs sign-in: only after Sign in; use Sign in on its page.
- Key not allowed / Vault locked: allow the key again or unlock the vault.
