# Vibe Work Debug API

Display names and their unchanged command IDs are listed in the generated [vocabulary reference](VOCABULARY.md).

The Debug API lets a local tool or agent control Vibe Work **as you**: everything you can see or do in the window
is readable and doable over HTTP on your own computer. It is how Vibe Work is tested end to end, and it is meant
for cross-agent work: one agent drives Vibe Work, reads what it shows, and gets screenshots of the window as
evidence.

**What turning it on means.** With the Debug API on, a connected agent can do what you can do in Vibe Work,
including the terminal and files, **except on your vault** (next section). Turn it on only for agents you would let
use your account. Vibe Work hides known secrets in its answers and screenshots where it can; that is a convenience,
not a guarantee.

**The vault is yours.** On the vault, a Debug API client has exactly the rights the agent has in Vibe Work, nothing
more. It can read what the agent can read: whether a vault exists and is unlocked, whether a key is set, the names of
the secrets agents may use, the website logins (name, site, user name), the "always" grants and the use log (never a
value or a password). Everything else is yours alone, in
the window, by hand:

- managing the vault: creating, unlocking or locking it, its password, "remember this computer";
- the Mistral key: setting, replacing, choosing a saved one, moving Vibe's own copy in, removing it;
- secrets and logins: adding one with its value or password (also as the new key of an MCP server), removing one;
- permissions: revoking an "always" grant, an MCP server's key grant or a login's fill grant, and removing an MCP
  server that uses a key from your vault (that drops its grant).

These requests answer `403` with the reason word `vaultUserOnly` and the message "The vault is managed in the window
by you; the Debug API has the agent's access here." The same holds for a command the window runs while it serves a
UI-control request (a `POST /ui/click` on Revoke or Unlock is the Debug API's act, not yours). Where the agent asks
you in a native dialog (storing a key captured from a page, giving an MCP server a key from your vault, filling a
vault login into a page), the Debug API
may start the same request and the same dialog appears; only your answer in the window decides, and no route can
answer it (`/ui/click` and `/ui/key` act inside the page, `POST /window` acts on the main window only). A program
running as your user on this computer (the terminal included) could still send keystrokes to the dialog with an OS
automation tool; that is outside what Vibe Work can prevent, the same as for any program you run. Why: the vault is
meant to become your main vault shared with your other apps, where no debug access exists.

What Vibe Work does: no route offers a stored secret as a value (your Mistral API key, the values in your Vault,
connector or MCP server credentials, the Debug API's own token); facts about them (a key is set, which vault item
holds it, the last test result) are readable. Before an answer leaves the app, any known secret value in it (as
text, JSON-escaped, percent-encoded or base64) is replaced by `[secret withheld: <kind>]`, and the answer carries
the header `x-vw-secret-withheld: <kinds>`. Screenshots paint key and password fields grey. A value that was
changed on the way (reversed or hex-dumped in a terminal, for example) is not recognised.

## Turning it on

- **Settings > Developer > Debug API.** The switch says what it does: "Lets a local agent do what you can do in Vibe Work, terminal and files included; it hides known secrets where it can, but that is no guarantee." (On the vault it has the agent's rights only, above.) A "Debug API on"
  badge shows in the window while it runs.
- Release builds start with it **off**; developer builds start with it on. Your choice in Settings is saved
  (`settings.json`, key `debugApi`) and wins over the build default. Off means no socket is open at all.
- Turning it on needs no confirmation: the token below is the gate, and only your user account can read it.

## Finding the port and the token

The Debug API listens on `127.0.0.1` only, on a random port per run. Vibe Work writes two files in its private
data folder, in the subfolder `debug-api/`:

| File | Content |
|---|---|
| `debug-api-<pid>.json` | `{url, port, tokenFile, pid, startedAt}` for this run (no token) |
| `debug-api.token` | the bearer token: 64 hex characters, readable only by you |

The data folder is the app data folder of `app.vibework.desktop`: `~/.local/share/app.vibework.desktop` on
Linux, `~/Library/Application Support/app.vibework.desktop` on macOS, `%APPDATA%\app.vibework.desktop` on
Windows. The token stays the same across restarts until you press **Regenerate token** in Settings.

Every request, including `/health`, needs `Authorization: Bearer <token>`. Browser-page requests are refused:
an `Origin` header is refused by default, and browser fetch metadata without one is also refused. The `Host`
header must name the listener: `127.0.0.1` or `localhost`, with this run's port or without one. Requests with
JSON bodies need `Content-Type: application/json`.

```sh
cd "$HOME/.local/share/app.vibework.desktop/debug-api" # Linux example; use the data folder above on other systems
PORT=$(jq -r .port debug-api-*.json)
TOKEN=$(cat debug-api.token)
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/health
```

Errors are `{"error": {"code", "message", ...}}` with a matching status (400 invalid, 401 token, 403 refused,
404 unknown, 409 conflict or stale, 428 a precondition such as `confirmDeletes`, 503 not available).

## What "as you" means

- Most Debug API requests count as your consent. <!-- vocabulary:fullAccess:start -->Full access<!-- vocabulary:fullAccess:end --> escalations still need your answer in the native
  dialog, as do protected vault and provider sign-in flows. A trusted native access-menu click
  is itself the decision to switch; `/ui/click` is synthetic and still asks. Actions are still labelled: logs, refusal events and the transcript say
  when the Debug API acted, and text a Debug API client typed is marked in the composer.
- No answer is shortened for the Debug API: paths, names, branch lists and inspections are what the window shows.
- The Debug API picks folders like **Choose folder** does (`pickFolder`).
- Rules that are not about you stay: the agent in a session is still limited to its session, pages and previews
  stay sandboxed, methods that are refused for everyone stay refused, and known secret values are hidden in
  answers where Vibe Work can (see the top of this page).
- A command the window runs for a `ui/*` request counts as the Debug API's only while that request is being
  served; the window cannot claim the Debug API's standing on its own.

## Routes

### `GET /health`

The API version, app and Vibe versions, uptime, why the API is on. Fields: `ok`, `apiVersion`, `appVersion`,
`vibeVersion`, `uptimeMs`, `enabledReason`, `debugApi`, and `rateLimits`: the last per-minute rate-limit headroom a
Mistral answer reported for your key (`tokensLimit`, `tokensRemaining`, `requestsLimit`, `requestsRemaining`,
`observedAtMs`; `null` before the first answer or without a key service). The numbers are per minute: about 60 s after `observedAtMs`
without a newer answer they are history, not the current headroom.

```sh
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/health
```

### `GET /server`

The Vibe app-server's state (running, restarting, `notInstalled` before Vibe is installed at setup, its last stderr lines with secrets replaced). While Vibe is not installed, every command that needs it answers 503 `serverUnavailable` with the reason word `notInstalled` at once.

### `GET /sessions`

The session list (`?limit=`, `?cursor=`, `?includeArchived=true`).

### `GET /sessions/{id}`

One session as the app holds it (state, history, turns).

### `GET /sessions/{id}/anomalies`

The session's protocol anomaly counters.

### `GET /sessions/{id}/processes`

The session's processes and listening ports (owner, evidence, leases).

### `GET /sessions/{id}/changes`

The Changes tab's view of the session (diff against its review baseline, or its turns' file edits). A Markdown folder link in an agent handover, such as `[Changes](git%20test/)`, opens this tab with that folder selected only when Git status discovered the folder as a repository.

### `GET /events`

Server-Sent Events: snapshots, then ordered changes and app events. `?session=<id>` for one session. Event
names:

| Event | What happened |
|---|---|
| `snapshot` | the full state of one session (replaces your copy) |
| `change` | an incremental change after the snapshot it follows |
| `lagged` | you fell behind and missed items; fresh snapshots follow |
| `server` | the Vibe app-server's state changed |
| `callbackOpened` | Vibe asks for an answer (approve, question); answer with `answerCallback` |
| `callbackAnswered` | a callback was answered, and by whom |
| `refused` | a command was refused: a gate or policy refusal, never a plain `notFound` answer (never with its parameter values) |
| `alert` | something the user must see at once |
| `sessionStuck` | a session could not be reopened after a restart (`retryReopen`, or continue in a new one) |
| `sessionHolder` | another process (the Vibe CLI, another app) holds a session a window shows (`holderPid`), or let it go (`null`): read-only here meanwhile; checked every 2 s |
| `debugApi` | the Debug API was switched on or off |
| `keyNeeded` | a model request waits for the Mistral key (`unlock` or `setup`) |
| `keyStatusChanged` | vault or key status changed: read `keys` `readStatus` |
| `vibeInstall` | the Vibe install changed (phase, progress with the last output lines, the offer, existing installs, Vibe updates): the whole install view |
| `rateLimits` | a Mistral answer reported the per-minute rate-limit headroom (same fields as in `GET /health`) |
| `terminal` | a terminal was opened, exited, restarted or closed |
| `documentsAdded` | files arrived through the native picker or a drop on the window |
| `documentChanged` | a document's state changed (reading, ready, failed) |
| `changesUpdated` | a session's changes differ from the last read: read them with `changes` |
| `previewChanged` | a session's preview changed: read it with `preview` |
| `previewShowRequested` | an explicit preview width asks the window to show that session's App preview in Browser; applies only to the selected session |
| `browserChanged` | a session's agent browser changed (the agent started or ended an action, Stop, Resume, the action log): read it with `browser` `state` |
| `secretsChanged` | project secrets, their grants or the use log changed: read them with `secrets` `list`; never a value |
| `pagesChanged` | a session's Project tab changed: read it with `pages` |
| `filesChanged` | a session's File tab viewer changed: read it with `files` `viewer` |
| `lineCommentsChanged` | a session's diff comments changed: read them with `lineComments` |
| `projectsChanged` | your project groups changed: read them with `projects` `list` |
| `worktreesChanged` | a session copy was created or removed: read the count with `worktrees` `count` |
| `dictation` | a running dictation's live text, or why it ended |
| `connectorSignIn` | a server sign-in ended (never with a token) |
| `marketInstall` | a marketplace install moved on (`download`, `unpack`, `setup`, `finish`) or ended (`installed`, `failed`, `cancelled`) |

```sh
curl -sN -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/events
```

### `POST /commands`

Runs one command, exactly as the window does (the list is below).

```sh
curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"type":"startTurn","sessionId":"<id>","message":[{"type":"text","text":"Hello"}]}' \
  http://127.0.0.1:$PORT/commands
```

The browser pool is also available through `POST /commands`: `{"type":"browser","request":{"op":"enginePool"}}`
reads the current snapshot; `setEnginePool` with `configuredParallelSessions` set to `auto`, `1`, `2`, `3` or `4`
saves the preference with user rights. Both return `result.type: "enginePool"` and `result.pool`.
See [Parallel browser sessions](BROWSER_POOL.md) for memory limits and release behavior.

Browser delegation permissions use the same user-rights command endpoint:
`{"type":"browser","request":{"op":"permissions"}}` reads them; `setPermissions` replaces
`permissions` with `{"closeUserTabs":false}` or `{"closeUserTabs":true}`. Missing or old values default to false.

<!-- vocabulary:browser:start -->
The window calls this choice **Close tabs you opened**.
For closing tabs you opened, **Ask me (default)** asks **Close** or **Keep** each time; **Allow** lets the agent close them during your task. These are `closeUserTabs: false` and `true` on the wire. App preview cannot be closed by the agent.
<!-- vocabulary:browser:end -->

`{"type":"browser","request":{"op":"state","sessionId":"<id>"}}` also returns `state.receipts`:
session id, completion time (`atMs`), site host, act, authority and outcome. A completed user-tab close records
`act: "closeUserTabs"` with `authority: "closeUserTabs"` for the standing setting or `"confirmedByYou"` for a
Close answer. The tool result says `closed by your permission` or `closed with your permission`, respectively.
These metadata-only receipts survive
app restarts, are removed when the session is deleted, and appear in the Browser panel's permissions activity.
They contain no page values, URL paths or queries. A declined confirmation leaves the ordinary
refused action-log entry; it does not claim a permission was exercised.

### `POST /callbacks/{id}`

Answers an open approval or question: `{"sessionId": "<id>", "output": {"type": "approval", "decision": {"type": "approve"}}}`.

### `GET /sessions/{id}/terminals` and `POST /sessions/{id}/terminals`

Lists or opens the session's terminals (`{"cols": 120, "rows": 30}`).

### `GET /terminals/{tid}/output`

Reads a terminal's output (`?from=`, `?maxBytes=`, `?plain=true` without escape codes).

### `POST /terminals/{tid}/input`

Types into a terminal: `{"data": "ls\r"}`. The Debug API writes with your terminal rights. With
`"binary": true`, each character from U+0000 through U+00FF is sent as one byte; it is not base64.

### `POST /terminals/{tid}/resize`

`{"cols": 100, "rows": 40}`.

### `POST /terminals/{tid}/restart`

Restarts the terminal's shell.

### `DELETE /terminals/{tid}`

Closes the terminal.

### `GET /windows`

The app's own windows: the main window first, then native dialogs the system lets the app see.

### `GET /screenshot`

A PNG of one of the app's own windows, never the screen or another app. `?window=main` (default) or a dialog id
from `/windows`; `?crop=<CSS selector>` cuts the main window to one element (`&pad=8` adds a margin).

```sh
curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/screenshot" -o window.png
curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/screenshot?crop=.composer&pad=8" -o composer.png
```

| System | Main window | Behind other windows | Minimized | Native dialogs |
|---|---|---|---|---|
| Windows | the client area with every webview (`PrintWindow`), converted to sRGB | captured (`PrintWindow` reads the window, not the screen) | restored for the capture, minimized again | the app's own dialogs (the folder picker, confirmations) with their frame |
| Linux | composed from the window's webviews (the main view, previews, the Project viewer) | captured (the webviews' own snapshots) | not specified | GTK dialogs of the app, drawn offscreen (without the header bar) |
| macOS | composed from the window's webviews | captured (the webviews' own snapshots) | not specified | not captured (that needs the screen-recording permission, which the app does not ask for) |

**Secret fields are blanked.** Before a main-window PNG leaves, every key or password field on screen is painted
over with flat grey; during the capture the fields show dots and their Show button is disabled, and when a secret
field moved or appeared while the window was captured the image is discarded (tried once more, then
`409 secretMoved`). The header `x-vw-secret-blanked` says how many fields were blanked. When the window's UI cannot
say where its secret fields are, the screenshot is refused (`503 secretCheckFailed`). Every route that takes a CSS
selector matches it as if every secret field were empty, so no selector can tell anything about a secret's value.

**No picture after a vault fill.** Once a vault login's password was filled into a page of a session's browser, every
picture of that page is refused until it loads another document: the main-window screenshot (the page is one of its
webviews), `preview` `screenshot`, `browser` `read` with `screenshot`, and the agent's screenshots and full-page
captures. The page could show the password in clear (a "show password" control); the text answers withhold it as a
known secret, pixels cannot be checked.

Wait for the window to settle before a screenshot: read `POST /ui/revision`, act, then `POST /ui/settle`.

### `POST /window`

The main window's size and state: `{"action": "state"}`, `{"action": "size", "width": 1280, "height": 800}`
(logical pixels; the minimum size still applies), `maximize`, `restore`, `minimize`, `focus`.

### `GET /ui`

The UI-control routes below, with their descriptions.

### `POST /ui/{name}`

Acts on the window's own UI state like a click or a key press does. The window answers; a request waits up to
10 s for it.

### Test builds only: `POST /e2e/script` and `GET /e2e/windows`

Present only in debug builds compiled with the cargo feature `e2e-script` (the end-to-end test builds); a release
build with that feature does not compile, so no released Vibe Work has these routes. `POST /e2e/script`
(`{"script": "<function body>"}`) runs the script in the main window's page and answers `{"value": ...}` (its
return value; `422 scriptError` when it throws, `504 scriptTimeout` after 30 s); `GET /e2e/windows` lists the
app's webview windows. They are how the window is tested where no WebDriver exists (macOS). The same token and
request checks apply, and answers pass the same secret check, but a script can read anything the page holds,
including what is typed in a key field before it is saved: never run such a build for everyday use.

## Commands (`POST /commands`)

Every command is `{"type": "<name>", ...}`. Requests with an `op` take `{"type": "<name>", "request": {"op": ...}}`.

| Type | What it does | Example |
|---|---|---|
| `listSessions` | the session list | `{"type":"listSessions","limit":20}` |
| `readSession` | one session | `{"type":"readSession","sessionId":"<id>"}` |
| `refreshSession` | reads it again from Vibe | `{"type":"refreshSession","sessionId":"<id>"}` |
| `startSession` | a new session | `{"type":"startSession","agentConfig":{"cwd":"<folder>"}}` |
| `resumeSession` | opens a stored session | `{"type":"resumeSession","sessionId":"<id>"}` |
| `forkSession` | branches a session | `{"type":"forkSession","sourceSessionId":"<id>"}` |
| `rewindSession` | rewinds to an entry | `{"type":"rewindSession","sessionId":"<id>","entryId":"<entry>","inplace":true}` |
| `startTurn` | sends a message | `{"type":"startTurn","sessionId":"<id>","message":[{"type":"text","text":"Hi"}]}` |
| `steerTurn` | adds to the running turn | `{"type":"steerTurn","sessionId":"<id>","expectedTurnId":"<turn>","message":[{"type":"text","text":"Also add tests"}]}` |
| `interruptTurn` | stops the running turn | `{"type":"interruptTurn","sessionId":"<id>","expectedTurnId":"<turn>"}` |
| `stopSession` | cancels compaction by closing this session's channel; preserves saved history and the next send reopens it | `{"type":"stopSession","sessionId":"<id>"}` |
| `pickModel` | model and thinking level (`reasoningEffort` on the wire) | `{"type":"pickModel","sessionId":"<id>","modelAlias":"<model>"}` |
| `setDefaultModel` | saves an offered model for future sessions without changing existing sessions; allowed through the Debug API as a user preference outside the vault | `{"type":"setDefaultModel","modelAlias":"<model>"}` |
| `setAccessMode` | the access mode; <!-- vocabulary:fullAccess:start -->Full access<!-- vocabulary:fullAccess:end --> (`auto-approve`) needs native confirmation | `{"type":"setAccessMode","sessionId":"<id>","agentName":"accept-edits"}` |
| `answerCallback` | answers an approval or question | `{"type":"answerCallback","sessionId":"<id>","callbackId":"<cb>","output":{"type":"approval","decision":{"type":"approve"}}}` |
| `listCallbacks` | the open callbacks | `{"type":"listCallbacks"}` |
| `requestConfirmation` | a single-use token for one exact call; <!-- vocabulary:fullAccess:start -->Full access<!-- vocabulary:fullAccess:end --> and protected consent flows show a native dialog | `{"type":"requestConfirmation","method":"session/delete","params":{"sessionId":"<id>"}}` |
| `call` | any other Vibe method the app allows | `{"type":"call","method":"session/delete","params":{"sessionId":"<id>"}}` |
| `viewSession` | which session this client shows | `{"type":"viewSession","sessionId":"<id>"}` |
| `retryReopen` | tries a stuck session again | `{"type":"retryReopen","sessionId":"<id>"}` |
| `cancelReopen` | stops that try | `{"type":"cancelReopen","sessionId":"<id>"}` |
| `continueInNewSession` | continues a stuck session in a new one | `{"type":"continueInNewSession","sessionId":"<id>"}` |
| `readStatus` | app status, versions, Debug API flag | `{"type":"readStatus"}` |
| `readAnomalies` | anomaly counters | `{"type":"readAnomalies","sessionId":"<id>"}` |
| `vibe` | installing Vibe at setup: `readStatus`, `prepare` (an offer: sources, checksums, the commands that will run, sizes, an `offerId`), `confirm` (`offerId`: the user's consent to exactly that offer; starts the install), `cancel`, `checkExisting` (finds Vibe installs already on the computer and reads their versions), `useExisting` (`path`, `acceptWarning` for another version). Vibe updates: `prepareUpdate` (the offer for the newer tested Vibe the last update check found in the signed supported-Vibe list: sources with checksums, the commands, sizes, an `offerId`), `confirmUpdate` (`offerId`: installs it next to the running Vibe; it starts with Vibe Work's next start), `cancelUpdate`; the view's `update` part says what the last check found (`available`, `upcoming`: newer on PyPI but not supported yet, never installed, `mismatch`: a notice for your own Vibe install, `checkError`). The check itself runs only with the app update check (`POST /ui/updates` `check`). Answers the install view; refusals are `vibeInstall` with a reason word (`wrongState`, `staleOffer`, `unsupported`, `unknownExisting`, `needsAcceptance`, `disk`, `noUpdate`, `notConfigured`) | `{"type":"vibe","request":{"op":"prepare"}}` |
| `keys` | the Mistral key and the vault: `readStatus`, `listKeyCandidates`, `testKey`, `detectVibeKey` (never answers a key). `createVault`, `unlockVault`, `lockVault`, `setupStoreKey`, `removeKey`, `useExistingKey`, `importVibeKey`, `removeVibeKeyOriginal` are the window's: refused here (`403 vaultUserOnly`, "The vault is yours" above) | `{"type":"keys","request":{"op":"readStatus"}}` |
| `terminal` | terminals: `open`, `list`, `write` (sends input with your rights), `resize`, `read`, `restart`, `close` | `{"type":"terminal","request":{"op":"list"}}` |
| `instructions` | the AGENTS.md files: `read`, `readCli` (reads the command-line copy), `write` (replaces the app or project file, comparing optional `expected` text to prevent stale edits), `apply` (reopens affected sessions so they take current instructions). These commands currently run with Debug API user rights. | `{"type":"instructions","request":{"op":"read","scope":"personal"}}` |
| `documents` | documents in the composer: `list`, `read`, `save` (`id`, `sessionId`: saves original bytes under the project attachments folder and returns an absolute `path`), `extract` (optional OCR), `remove`, `takePicture`, `addDropped` (files dropped on a composer, base64) | `{"type":"documents","request":{"op":"list"}}` |
| `files` | the File tab: `resolve` (paths as the chat wrote them, in the session folder or a picked folder), `read`, `stat`, `view`, `layout`, `reload`, `viewer`, `clearExternal`, `screenshot`, `open` (default app; never a program), `reveal`, `write` (a quick edit: the whole new text and the `fingerprint` it started from; `stale` when the file changed on disk) | `{"type":"files","request":{"op":"resolve","sessionId":"<id>","paths":["src/main.rs"]}}` |
| `speech` | dictation and speech: `start`, `append`, `stop`, `cancel`, `speak`, `voices` | `{"type":"speech","request":{"op":"voices"}}` |
| `changes` | the Changes tab: `read`, `keep`, `revert`, `restart`, `previewUndo`, `folderSessions`, `planHistory`, `enableHistory` (`confirmed: true`), `dismissOffer` | `{"type":"changes","request":{"op":"read","sessionId":"<id>"}}` |
| `lineComments` | comments on diff lines: `list`, `add`, `remove`, `markSent` | `{"type":"lineComments","request":{"op":"list","sessionId":"<id>"}}` |
| `preview` | the pinned App preview: `list`, `open` (this session's local servers only), `back`, `forward`, `reload`, `close`, `setWidth`, `layout`, `browserOverlay`, `screenshot`, `console`, `explain`, `pick`, `cancelPick`, `stopOwner` | `{"type":"preview","request":{"op":"list","sessionId":"<id>"}}` |
| `browser` | the agent browser of a session: `read` (the page as the agent reads it: refs, a page of text, prompt-guard flags; `maxRefs`, `textPage`, `screenshot`; `guardLayers` names the address checks that apply to every request of the page besides the navigation rules: `requestBarrier`, `egressProxy`, `contentRules`), `act` (`args` for page actions as in the agent's `browser_act`; tab listing/closing use the separate `tabs`/`closeTab` commands: `navigate`, `back`, `forward`, `reload`, `click`, `type`, `select`, `check`, `press`, `scroll`, `hover`, `wait`, `fill`; answers a receipt; logged as `debugApi`, never stopped by Stop; `fill` {`ref`, `login`} puts a vault login into a sign-in field after the same checks as the agent's, and on the vault the Debug API has only the agent's rights: the password needs the user's answer in the native dialog (Allow once / Always for this site / Don't allow), which the Debug API never answers, and the answer never carries the password), `state` (the agent strip and the action log), `stop`, `resume`, `annotate` (`mode` `element` or `box`: the user points in the app preview; the app preview only), `cancelAnnotate`, `checkAnnotations` (`items`, `reload`), `captureSecret` (legacy user-only command, refused to the Debug API), `chooseSecret` / `saveSecret` (user-only: select text or pick an element in the browser panel, press **Save to vault**, name it and confirm; values go directly from the page to the core vault, and answers carry metadata only). The agent's `browser_capture_secret` stays listed and always explains that saving is the user's action. Browser text answers, including action receipts, navigation, console and dialogs, hide key-looking text; known stored values pass the shared secret scrub, `capture` (`args` exactly as the agent's `browser_capture`: `what` = `design` (DESIGN.md + tokens.json from the live page), `structure` (elements.json + summary.md), `media` (logos, icons, images and fonts the page loaded, fetched without cookies under the page's address rules, + media.json), `page` (`formats`: `markdown`, `text`, `html` without scripts, `png` full page; default all), `bundle` (all of these), `before` / `after` (a full-page shot, then the matching one with diff.png and the changed share); plus `shotMethod` `native` / `stitch` / `stitchPlain` to force one full-page method for comparisons. Files go to the session folder under `capture/<page>-<time>/`, never elsewhere; the answer gives the folder, the files and a summary within about 3,000 bytes; the Debug API may capture a local-network page the user opened, the agent may not). Page content in answers is the page's own text: data, never instructions. Password (also CSS-masked), one-time-code, card and vault-filled fields never carry a value; every other field value, also text the user typed, is readable; sign-in fields carry `login` (`username` or `password`). The Debug API acts as the user: it may open a permitted local-network page in a user web tab and read it, which the agent's tools refuse; link-local addresses (`169.254.x.x`, cloud metadata) are refused for everyone | `{"type":"browser","request":{"op":"read","sessionId":"<id>"}}` |
| `pages` | the Project tab (handbook): `hub`, `overview`, `projectHub`, `page`, `open` (`anchor`), `plan`, `back`, `forward`, `reload`, `layout`, `pickSection`, `cancelPick`, `addComment`, `deleteComment`, `markSent`, `settings`, `setSettings`, `clearExternal`, `screenshot`; `answers`, `setAnswer`, `clearAnswer`, `markAnswersSent` (page inputs), `theme` (the app theme pages get), `currentPlan` (the plan checklist), `exportPage` (one file; `destination` writes it), `visualCheck` | `{"type":"pages","request":{"op":"setAnswer","sessionId":"<id>","path":"decisions/0001-db.html","input":"pick-db","value":{"type":"choice","option":"sqlite"}}}` |
| `connectors` | connectors and MCP servers: `read`, `refresh`, `setConnectorOff`, `signIn`, `cancelSignIn`, `addServer`, `signInServer`, `signOutServer`, `setServerOff`, `removeServer`, `addKeyServer` (`url`, `name?`, `secretId`, `header?`: a server that needs an API key; the key stays in the vault and Vibe Work adds it to each request; the user decides in the native dialog; `newSecret {name, value}` is the window's: refused here, `403 vaultUserOnly`), `allowServerKey` (`name`; the native dialog decides). `removeServer` of a server that uses a key from the vault is refused here (`403 vaultUserOnly`) | `{"type":"connectors","request":{"op":"read"}}` |
| `marketplace` | the Tools marketplace (reviewed, pinned catalog): `read`, `install` (also updates; runs in the background, follow `marketInstall` events), `cancelInstall`, `remove` | `{"type":"marketplace","request":{"op":"install","id":"context7"}}` |
| `libraries` | your Mistral document libraries: `list` (with scopes and use facts), `create` (`name`, optional `description`, and a `scope` object such as `{"kind":"off"}`, `{"kind":"global"}` or `{"kind":"project","projectId":"<id>"}`), `setScope` (`libraryId`, `scope`), `delete` (on Mistral, with its documents), `documents`, `upload` (files as base64; they go to Mistral), `deleteDocument`, `text` (the extracted text a source opens), `ask` (`question`, `libraryIds`, with an empty list meaning all enabled libraries) | `{"type":"libraries","request":{"op":"list"}}` |
| `images` | `read` returns image generation costs, limits and the number of requests made this app run. There is no separate image approval switch or `allow` operation; the agent's image tool follows the normal tool approval flow. | `{"type":"images","request":{"op":"read"}}` |
| `worktrees` | a session in its own copy: `options`, `suggestName`, `start`, `info`, `inspect`, `remove`, `keep`, `orphans`, `count` | `{"type":"worktrees","request":{"op":"count"}}` |
| `git` | the Changes tab's git loop: `status`, `prepareCommit`, `draftMessage`, `commit`, `pushPlan`, `push`, `prPlan`, `createPr`, `prStatus` (`refresh` asks the forge), `prepareMessage` (`checks`, `review`, `updateFromBase`; the text, never sent), `bringPlan`, `bringIn`, `settings`, `setSettings`. Every request accepts `repo`, a repository root relative to the session folder (`.` for the session repository, e.g. `sub` for a nested repository). Status returns `repos` (name, root, branch) and the chosen `repo`; omitted `repo` selects the session repository when present, otherwise the first discovered repository. `push`, `createPr` and `bringIn` show no dialog for the Debug API (your request is the consent) and run only what the plan named | `{"type":"git","request":{"op":"status","sessionId":"<id>","repo":"sub"}}` |
| `projects` | your project groups: `list`, `refresh`, `create`, `rename`, `setFolder` (`folder: null` clears it), `delete` (sessions stay), `move`, `place`, `start` (requires `projectId` and `agentConfig`; starts a session in a project) | `{"type":"projects","request":{"op":"list"}}` |
| `projectInstructions` | Project settings' instruction-file commands are user-only: every operation is refused to the Debug API. Use the desktop app to list, create, edit, link or delete these files. Agents can edit project files with their own filesystem tools. | unavailable through the Debug API |
| `processes` | `leftRunning` (processes a deleted session left running), `afterDelete` (the window's question after a delete; the Debug API gets the list), `stop` (one listed `id`); `session` lists one active session's processes, `output` reads one process's recent output and `stopSession` stops one, each with `sessionId` and an `id` from that list where needed | `{"type":"processes","request":{"op":"session","sessionId":"<id>"}}` |
| `secrets` | `list` returns secret, login, grant and use metadata without values; `listForProject` also returns each entry's permission for a `project` folder. All other operations, including `setEntryPermission` (`project`, `id`, `permission`), adding, generating, renaming, replacing, copying, removing, revoking and changing agent access, are refused to the Debug API (`403 vaultUserOnly`). Use the window for those choices. | `{"type":"secrets","request":{"op":"listForProject","project":"<folder>"}}` |
| `safety` | the safety reminders the agent's commands are checked against: `read`, `setRule` (`id`, `on`), `addCustom` (`text`: a command that contains it is stopped), `removeCustom` (`id`). Reminders against mistakes, not a sandbox | `{"type":"safety","request":{"op":"read"}}` |
| `pickFolder` | chooses a folder like Choose folder (Debug API only) | `{"type":"pickFolder","path":"<absolute folder>"}` |
| `search` | search inside messages and files (the same results Ctrl K shows): `messages` (every conversation's messages, or one with `sessionId`; the first search fills the index from Vibe's stored history, `index.filling` says it is still running), `files` (names and lines in the sessions' folders, or one session's; ignored and build folders skipped), `status` | `{"type":"search","request":{"op":"messages","query":"rate limit"}}` |
| `sessionTools` | a session as Markdown and temporary chats: `export` (the whole session as Markdown; tool steps summarised, file changes as diffs, known secret values replaced; `destination`, an absolute path, writes the file, else the text comes back), `startTemporary` (a chat Vibe Work deletes when it quits), `temporary` (their ids), `holder` (which other process holds a session now, Vibe 2.26.1), `setSubagents` (`enabled`: saves the Subagents setting for new chats through a short-lived empty chat that is deleted again; chats that are already open keep their value, and the answer's `leftoverSessionId` names that chat if it could not be deleted) | `{"type":"sessionTools","request":{"op":"export","sessionId":"<id>","destination":"<absolute path>.md"}}` |

Project entry permissions are `askEachTime`, `allowed` and `never`. `secrets.listForProject` accepts an existing
project folder, and the user sets an individual secret or login's choice in the window with `setEntryPermission`.
A Debug API client may read the choice but cannot change it. See the [vocabulary reference](VOCABULARY.md) for
the displayed names.

The `browser` command also has user-only Settings account controls: `accounts` returns provider sign-in hints
without cookie values; `signIn` and `signOut` take a `provider`; `accountsLayout` takes an optional `rect` to
place or hide the sign-in page; `closeSignIn` closes that page without clearing its provider sessions. The
sign-in page is reserved for direct user input, and normal browser `read` or `act` requests cannot target it.
For a focused browser read, `read` accepts `find` and `maxBytes` as well as `maxRefs`, `textPage` and `screenshot`.

### Browser page tabs

The Browser panel has a pinned **App preview** and separate web tabs. The app preview opens this session's local
servers and keeps annotations. Agent browser tools use the agent's current web tab; they never navigate the app
preview. `preview.open`, `preview.back`, `preview.forward`, `preview.reload`, `preview.close` and `preview.screenshot`
address the pinned app preview. `preview.layout` accepts an optional `tabId` to display the selected web tab in the
native page area; omitted `tabId` addresses the app preview when it is selected. Layout does not change selection,
and a stale layout for a different tab is ignored. A hidden panel never makes a native page visible.

`POST /commands` with `type: "browser"` exposes these user-rights requests (also to the Debug API):

| Operation | Request fields | Result |
| --- | --- | --- |
| `tabs` | `sessionId` | `tabs`: pinned app preview first, then ordered web tabs |
| `openTab` | `sessionId`, optional `url` | Opens a user-controlled web tab; `tabs` |
| `switchTab` | `sessionId`, `tabId` | Selects and displays an owned tab; `tabs` |
| `closeTab` | `sessionId`, `tabId` | Frees a web tab's engine; pinned app preview is refused; `tabs` |
| `reorderTabs` | `sessionId`, `tabIds` (every web tab exactly once) | Reorders web tabs after the pinned preview; `tabs` |
| `reviewHandoff` | `sessionId`, `tabId` | Current page title, URL, site, owner and fingerprint |
| `handoffTab` | `sessionId`, `tabId`, reviewed `fingerprint`, `confirmed: true` | Gives the agent this exact user tab if the page and ownership are unchanged; `tabs` |
| `takeBackTab` | `sessionId`, `tabId` | Cancels pending agent input and returns a delegated tab to user control; `tabs` |

Each tab carries `pageKey`, `kind` (`appPreview` or `web`), `ownerKind` (`user`, `agent` or
`delegatedToAgent` for web tabs), `active`, `agentCurrent`, `preview` (title, URL, loading and live/hidden/suspended
state), an optional prior visible `snapshot`, and optional `faviconUrl`. A user-created tab stays under user control
until the reviewed handoff; handoff grants no vault permission. The Debug API acts with user rights and may review,
confirm and take back a handoff. A review becomes stale if its page, mount or owner changes.

`browser.read` and `browser.act` accept optional `tabId` and default to the user's selected tab, including the app
preview. For selected web tabs, `browser.act` with `navigate`, `back`, `forward` or `reload` drives that page; action
receipts and reads include `tab.id` and `tab.site`; the page-derived title and full URL are under
`untrustedPageContent.tab`. `browser.capture`, `browser.chooseSecret`,
`browser.saveSecret` and `browser.cancelSecret` accept optional `tabId` as well. Annotation and annotation rechecks
always address the pinned app preview.

Agent `browser_read`, `browser_act` and `browser_capture` accept optional `tab`: omitted means the agent's current
web tab, `"new"` creates a new agent tab, and an opaque id selects a permitted web tab. Agent tools refuse the pinned
app preview and user-controlled tabs. Stop applies to every agent web tab in the session, including newly created
tabs, until Resume or the next turn. The agent's `preview_open` still addresses the pinned app preview.
Agent `preview_list`, `preview_open`, `preview_screenshot` and `preview_console` identify that target as
`tab: {id: <sessionId>, kind: "appPreview", label: "App preview"}`.

The window's `browser.cancelSecret` (`sessionId`, `handle`) discards only the named
pending choice and returns `done`. It is UserOnly like choosing and saving;
Debug API calls are refused before accessing the page. An open name prompt has
no fixed choice expiry; finishing or cancelling releases its eviction guard.

## UI-control routes (`POST /ui/{name}`)

### `POST /ui/browser`

The Browser tab's agent browser: the agent strip (agent active, acting, stopped), the action log, the address bar, the notes on the app preview and the Capture menu, every action the tab has. Body: {action: state|tabs|openTab|switchTab|closeTab|stop|resume|annotate|cancel|note|remove|send|recheck|address|capture, sessionId?: string (default: the selected session), ...}: state = the open URL, the ordered tabs and selected tab, whether notes are offered (pages of the session's own local servers only), the strip, the log (who: agent|debugApi|user, action, target, outcome, detail), the notes (id, mode, role, name, route, note, status draft|sent|changed|unchanged|missing, source hint, whether a crop exists, by ui|debug-api: a note a Debug API request created or edited stays `debug-api`), a running pick and the last error; tabs refreshes the tab list; openTab {url?} creates a user-controlled web tab; switchTab {tabId} selects an owned tab; closeTab {tabId} closes a web tab (the pinned App preview is refused); stop / resume = the strip's buttons (Stop cancels the agent's browser action in flight and refuses more until Resume or the session's next message); annotate {mode: element|box} starts a pick in the preview (a person clicks an element or drags a box; a client can click with `POST /commands` `{"type":"browser","request":{"op":"act",...,"args":{"action":"click","ref":"<ref>"}}}`) and answers the new note; cancel stops a pick; note {id, text}; remove {id}; send puts every draft note into the composer (a context chip and a cropped picture each, never a command; marked as the Debug API's when the send or the note came from it, also when the user later sends such a note from the window) and answers how many; recheck {reload?: bool} reloads the preview and sets each sent note to changed / unchanged / missing; address {url} opens a URL in the selected web tab with the user's navigation rights; from App preview it opens a local server there or creates a web tab for a public URL; capture {kind: design|structure|media|page|markdown|text|html|png|bundle|before|after} does what the Capture menu's item does and answers `captured` (folder, path, outcome, files count, fileNames array of receipt filenames, skipped count) with the state, whose `capture` part says what is running, the last result and the last error.

### `POST /ui/changes`

Read the Changes tab of the shown session and move around in it like a user. Body: {action: state|reload|select|layout|comment|removeComment|sendComments, path?, layout?: split|unified, line?, endLine?, side?: new|old, text?, send?, id?, ids?}: state = mode (none|git|effects|unavailable), whether the tab is offered, the files with counts, notes and turn labels, the shown file's hunks (ids), turns (folders without git), the open confirmation, the diff comments; select shows a file; comment {path, line, endLine?, side?, text, send?} comments on lines of the current file (send puts a context chip into the composer, never a command); removeComment {id}; sendComments {ids?} puts the unsent comments into the composer as chips; layout switches split/unified; reload reads again. Keep and revert are not offered here: use POST /commands with {type: changes} (origin debug-api).

### `POST /ui/click`

Clicks an element like the mouse does (every control is reachable, also those without a route of their own). Body: {selector: CSS selector, index?: n-th match}. A hidden or disabled element is refused (denied). Answers right after the click; an action that waits (a native folder picker) keeps running.

### `POST /ui/composer`

Read or act on a session's composer like typing and clicking do. Body: {action: read|type|send|insertContext|attachImage|stop|retry|slash|access|model|queueEdit|queueRemove|rewind|rewindClose|branch, sessionId?: string (default: the selected session, else the new-chat view's composer "new-chat")}: type {text}; send {mode?: send|steer|queue} sends the current content (a typed !cmd runs a shell command, a GUI /command runs it; on "new-chat" it first starts the session in the folder chip's folder, applies the held access and model, then sends the first turn, and outcome.started names the new session; without a folder it refuses and keeps the text); insertContext {label, source, text} adds a context chip (always data, never a command); attachImage {name, data: base64}; retry {instructions?}; slash {name, args?}; access {mode} and model {alias?, effort?} answer with the composer as it then reads (on the new-chat composer they are held until its first send); queueEdit {queueItemId, text}; queueRemove {queueItemId}; rewind {entryId, confirm?: here|new}; branch {entryId}. The read answer's `heldByPid` is the pid of another Vibe process (for example the Vibe CLI) holding the session, while the composer is read-only, else null.

### `POST /ui/connectors`

Read or act on the Connectors page like clicks do. Body: {action: read|open|close|filter|search|select|refresh|signIn|stopWaiting|turnOff|turnOn|addServer|signInServer|signOutServer|removeServer|serverOff|serverOn|addKeyServer|allowServerKey, alias?, name?, url?, secretId?, newSecret?, header?, filter?: all|connected|signin|off, text?}: read (default) returns whether the page is open, the filter, the groups with each connector's status, the servers, the sign-in in progress (connector or server, the page address, waiting) and the selection; signIn {alias} starts a Mistral connector sign-in through the selected (else newest) session and opens the link dialog with the page address; addServer {url, name?} adds a server that signs in with OAuth (refused addresses are refused before any dialog); addKeyServer {url, name?, secretId | newSecret {name, value}, header?} adds a server that needs an API key and allowServerKey {name} allows its key again: these two always show the native dialog in the window, also for the Debug API, because they give a server a secret; the user's answer decides (always, the recommended choice and the dialog's default button; allow until Vibe Work closes; or not). A `newSecret` with a value, and `removeServer` of such a server, are the user's alone: refused here (`403 vaultUserOnly`, "The vault is yours"). Other changes run without the native confirmation (the Debug API acts as the user); sign-ins finish only in a browser. Server entries carry `key` (the secret's name, the header and the grant), never a value.


### `POST /ui/marketplace`

The Tools page's Marketplace tab (plugins, skill packs and MCP servers from the reviewed, version-pinned catalog). Body:
{action: read|open|connectors|reload|filter|search|select|install|cancel|remove|addServer, id?, filter?:
all|installed|updates, text?}: read (default) returns the tab, filter, selection, note, the install in progress, the
platform and every shown entry with its status words, installed version, update flag, access class (servers) and its
state in the viewed session (loaded, skills, Vibe's issues); open / connectors switch the tab; install {id} installs or
updates a pack or plugin (in the background; progress arrives as `marketInstall` events); cancel stops it; remove {id}
removes it; addServer {id} adds a catalog MCP server through the Connectors flow (its sign-in finishes in a browser).
Example: `{"action":"install","id":"context7"}`.

### `POST /ui/documents`

Read or act on the document chips like clicks do. Body: {action: read|extract|remove|preview|asPicture, sessionId?: string (default: the selected session), docId?: string}: read returns each session's document chips (id, name, kind, status, pages, chars, kept, error) and the open preview; extract {docId} reads it with Mistral OCR (spends OCR quota); remove {docId}; preview {docId, open?: bool}; asPicture {docId} puts a PNG/JPEG/WebP/GIF into the message as a picture. To add a file, use ui/files drop (the same routing as a drop on the composer).

### `POST /ui/files`

The File tab (files the agent mentions, previewed) of a session, every action a click has (the Debug API acts as the user, no native dialogs). Body: {action: state|refs|open|switch|close|reload|mode|external|edit|drop, sessionId?: string (default: the selected session), ...}: state = the shown file (name, path relative to the session folder), kind, size, status, mode, truncation, the text it shows (up to 65,536 characters), the viewer and the recent list, and the last drop report; refs {entryId?} = the file references the transcript shows (text, path, line, state ok|no|pending); open {text, line?} resolves a path as the transcript does (the window's scope: the session folder and picked folders), or open {file} a handle from refs or the recent list; switch {file}; close; reload; mode {mode: rendered|source}; external {what: open|reveal} opens with the default app or shows in the folder (a program is never opened); drop {files: [{name, data (base64), folder?}]} routes files like a drop on the composer and answers the drop report and the composer's pictures, context chips and document chips. Known secrets in a file's text are hidden like in every answer (see the top of this page). edit {text} saves the shown text file with that whole text like the editor's Save: refused with `stale` when the file changed on disk since the tab read it; line endings and a byte-order mark are kept; the agent gets a context chip with the diff.

### `POST /ui/git`

The branch bar at the top of the Changes tab, the Commit / Pull request / Bring into main dialogs and the PR card of the shown session, every action a click has. Body: {action: state|reload|selectRepo|openCommit|setMessage|toggleFile|runHooks|commit|push|openPr|setPr|createPr|refreshCard|sendChecks|sendReview|openBring|bringIn|askVibe|close, sessionId?, repo?, text?, path?, on?, then?: none|push|pr, remote?, title?, body?, base?, draft?}. `selectRepo` picks a root returned by status (for example `sub`). `state` (default) returns the selected `repo`, discovered `repos`, whether the bar shows, the status (branch, base, upstream, remotes, files to commit, why Commit is blocked), the PR card, the open dialog (commit: the message and where it came from, the files with `included`, the author, the repository's commit hooks and `runHooks`, `credit`; pr: the plan, title, body, base, draft; bring: the dry run), busy, the last notice and what the last commit printed. Every action uses the selected repository. `openCommit` drafts a message (Mistral with your key, else the session title); commit {then?} commits the included files and can continue with a push or the PR dialog; push, createPr and bringIn carry the Debug API origin, so the core's confirmation is answered as yours and only what it names runs; sendChecks, sendReview and askVibe put a prepared message into the composer as a chip, never sent.

### `POST /ui/key`

Presses a key like the keyboard does. Body: {key: e.g. Escape|Enter|k, selector?: CSS selector to focus first, ctrl?, shift?, alt?, meta?}. Without a selector the key goes to the focused element.

### `POST /ui/layout`

Automatic layout checks of what is shown (the visual pass). No body. Returns the viewport, every shown menu, listbox, dialog, tooltip and popover with whether it is fully inside the window, buttons and headings whose text is cut off, horizontal scroll, and a problems list.

### `POST /ui/libraries`

Read or act on the Libraries page like clicks do. Body: {action: read|open|close|select|create|scope|delete|upload|deleteDocument|text|closeText|ask, libraryId?, documentId?, name?, scope?: off|global|project:<id>, files?: [{name, data (base64)}], question?}: read (default) returns whether the page is open, the libraries (id, name, documents, scope: who may use it in chats), the sentences about what is sent and what Mistral charges, the selected library's documents (id, name, Mistral's processing status, pages), the note, the last answer with its numbered sources and the open document text (its start); create {name, scope?} makes a library on Mistral (default scope global); upload sends the files' bytes to Mistral (like a drop; there is no path action, the window's Add files uses the native picker); delete deletes the library and its documents on Mistral; ask {question} asks across every library that is not off (spends Mistral calls). No native dialog (the Debug API acts as the user).

### `POST /ui/list`

Show, hide or toggle the session list. Body: {action: show|hide|toggle}. Returns whether it is visible.

### `POST /ui/notifications`

Read or set the notification state. Body: {action: read|level|focus}: level {level: errors|permissions|all}, focus {focused: bool|null} overrides the window-focus signal for tests (null = real). Returns the level, the focus override and the OS notifications the UI asked for since the page loaded. Each request contains a short body summary and may include a `notificationId` linking activation to the complete message retained in the app's Notifications panel.

### `POST /ui/pages`

Alias of ui/project for one release (the Project tab was called Pages in earlier versions). Same body and answer.

### `POST /ui/palette`

The command palette, like the keyboard uses it. Body: {action: list|open|close|run, query?, id?}: list (default) returns the commands (id, label, section) the query matches; open/close show or hide it; run {id} runs one exactly as Enter does.

### `POST /ui/panel`

Open, close or toggle the side panel and pick its tab. Body: {action: open|close|toggle, tab?: project|plan|browser|changes|terminal|file} (pages = project, an alias for one release). Returns the panel state.

### `POST /ui/plan`

The Plan tab (the plan checklist beside the chat) of the shown session. Body: {action: state|open, step?}: state (default) returns the current plan page (path, title), its steps (id, status, text), progress (done, total, doing) and whether the tab is open; open {step} does what a click on a step does (the Project tab at that step of the plan page).

### `POST /ui/processes`

The "Left running" list: processes the agent started that keep running after their session was deleted. Body: {action: read|open|close|stop, id?}: read (default) returns {open, items: [{id, sessionId, sessionTitle, pid, name, ports}], stopping}; open/close show or hide the list dialog like the status bar button and Close do; stop {id} stops one listed process like its Stop button (no dialog: the request is the click).

### `POST /ui/project`

The Project tab (the project handbook) of the shown session. Body: {action: state|hub|open|plan|back|forward|reload|pick|comment|send|delete|point|answers|answer|sendAnswers|check|export, ...}: open {path, anchor?} (anchor: a section or step id to scroll to); comment {path, section, text, quote?, send?}; send {id}; delete {id}; point {source: terminal|preview|page} ("Point Vibe at this": a context chip in the composer, never a command); answers {path?} (the page's inputs with the user's answers; every page with inputs without path and no page shown); answer {path?, input, value} (value: {type: choice, option} | {type: checklist, checked: [...]} | {type: rank, order: [...]} | {type: text, text} | null to clear; checked against the page file); sendAnswers {path?} (every answer of the page into the composer as one chip, never a command); check {path?} (the visual check now: desktop 1280 and phone 390 px, light and dark); export {path?, destination?} (the page as one self-contained HTML file: its text, or written to the absolute destination like the save dialog). path defaults to the shown page. Calls carry the Debug API origin (a label: no native dialog). Returns the tab state (hub with manifest, findings counts and groups, or page with steps, sections, comments, answers, visual check, prototype; the viewer) and the action's result.

### `POST /ui/projects`

The Projects overview: every project's handbook manifest, active plans and health without opening a session. Body: {action: state|open|close|refresh|select|filter|query|openHandbook|start|migrate|newSession|reveal|chooseFolder|newProject, ...}: select {projectId|null}; filter {status: all|active|maintenance|paused|archived}; query {text}; openHandbook {projectId, path?} (a session's Project tab); start {projectId} reuses the project's current chat (or starts one) and sends the handbook prompt as a user message, leaving any composer draft untouched. While the chat is working, the handbook request is unavailable; migrate {projectId} starts a session and sends a fixed migration prompt (the agent writes the files); reveal {projectId} shows the project folder; chooseFolder {projectId, folder} sets a folder (a path, or null to clear; `folder` is required: the native picker cannot be answered remotely); newProject {name} creates a project. Returns the page state (cards with name, state, status, plans, health).

`newSession {projectId}` acts like the sidebar's **New chat in this project**: creates a session in the project's
main folder, files it directly into that project, expands the group, selects the session and focuses its composer.
No message is sent. A project without a main folder uses the native folder picker; cancelling leaves selection and
folds unchanged. The response includes `session` (the selected session id) and the page state. These actions accept
`project` (the main folder) as an alternative to `projectId` and run with the Debug API's user rights.

### `POST /ui/rail`

Read or press the icon rail. Body: {action: read|home|newChat|projects|tools|libraries|settings}: read (default) returns the items top to bottom (id, label), the current one, whether a page lies over the work view, where Home goes (the selected session id and title, or null for the new-chat view) and the new-chat view's folder chip; home leaves Settings, the Projects overview, the Tools page and the Libraries page and shows the work exactly as it was (never a new session); newChat opens the new-chat view with its composer focused (no session exists until its first send, see composer); projects, tools, libraries and settings open those pages as their buttons do.

### `POST /ui/rect`

An element's rectangle (used by GET /screenshot?crop=). Body: {selector: CSS selector}. Returns {found, x, y, width, height, viewportWidth, viewportHeight} in CSS pixels.

### `POST /ui/revision`

The UI mutation counter (ShellX uiRevision). No body. Returns {revision, quietForMs, animations}: read it before an action and wait until it moved and the UI is quiet before a screenshot.

### `POST /ui/search`

Search like Ctrl K and Ctrl F do. Body: {action: palette|results|run|find|next|previous|findClose|fileFind|fileNext|filePrevious|fileFindClose|findState, query?, id?, sessionId?}: palette {query} opens the global search with the words typed; results (default) returns the rows the palette shows now (id, kind: session|message|file|setting|action, title, path, opens); run {id} opens one like Enter (a message opens its conversation at the match with the find bar, a file opens the File tab at the line); find {query?, sessionId?} is Ctrl F in a conversation (the find bar; sessionId defaults to the open session), next/previous step through its matches, findClose closes it; fileFind/fileNext/filePrevious/fileFindClose do the same in the File tab; findState returns both bars (session, query, current match) and their match counts.

Plain text shows Sessions, Messages, Files, then Settings in fixed groups. Start the query with `/` to search actions
instead; an empty query lists actions and sessions. Message and file searches wait 150 ms after typing and update
independently. Surviving rows retain their order and highlighted identity while newer results arrive. When no content
matches, the palette says so even if Settings has a match; pending or failed searches have their own feedback.

### `POST /ui/secrets`

Where the secret fields (key and password fields) are on screen. Body: {lock?: bool}: true turns the capture lock on (fields show dots, Show is disabled), false answers and then releases it (GET /screenshot does both around a capture). Returns {rects, clear, locked, epoch, viewportWidth, viewportHeight} in CSS pixels, never a value.

### `POST /ui/session`

Select a session, or the new-chat view (null). Body: {sessionId: string|null}. Returns the selected id.

### `POST /ui/sessions`

Act on the session list like a click does. Body: {action: read|filter|archived|more|refresh|open|rename|pin|archive|delete|retryReopen|cancelReopen|continueNew|temporaryChat|toggleGroup|projectCreate|projectRename|projectDelete|projectMove|projectPlace|projectStart|projectFolder|projectsRefresh, ...}: filter {text}, archived {show: bool}, more (load the next page), refresh, open/pin/archive/delete/rename {sessionId, ...} (pin {pinned: bool}, archive {archived: bool}, rename {title}); retryReopen, cancelReopen and continueNew act on a stuck session; delete runs without the native confirmation (the Debug API acts as the user) and also answers leftRunning: the processes the deleted session left running (they are not stopped, use ui/processes stop). temporaryChat {on: bool} sets the new chat's "Temporary chat" switch. The project actions create, rename, remove, move or place groups and sessions, start a chat in a project, choose its folder and refresh groups; each uses the same project id or name fields as the matching action in the window. Returns the list model (groups, pinned, needs-you, unread flags, badges), the temporary session ids as `temporary` and the switch as `newChatTemporary`.

`projectStart {projectId}` uses the same project chat action as `ui/projects` → `newSession` above and returns
`sessionId` (null on cancellation/failure) with the updated list model.

### `POST /ui/settings`

Read or act on the Settings page like clicks do. Body: {action: read|open|close|category|search|statusUsage|instructions|errorDetail|cliSharing}: read (default) returns whether the page is open, the category list and the shown category, the search inside Settings and its results (id, category, label, path), the shown category's rows as the user sees them (id, label, value: a switch's state, a pressed segment or a selector's choice, lit), the About facts (version, author, contact mailto: target, source link and project page targets), the personal instruction editor (document facts, draft text, dirty, review step, busy, notice, error) and the error-detail preference; cliSharing {share?: bool} saves the CLI session-sharing switch when `share` is given and returns it with the CLI home and both Vibe versions (applies at the next start); open {category?, setting?} shows the page at a category (with `setting`: at that setting, lit for a moment), close hides it (the view underneath shows again unchanged); category {id} shows one category; search {query} types into the search inside Settings; statusUsage {show?: bool} reads or sets the Appearance switch for the status bar's context readout; instructions {do: load|type|review|back|discard|save|loadCli, scope: personal, text?} edits the personal AGENTS.md through the same review step the screen has (save writes the reviewed draft only, and then makes open sessions reopen with it); errorDetail {mode: plain|details}.

### `POST /ui/settle`

Waits until the UI is quiet. Body: {quietMs?: 250, timeoutMs?: 5000 (max 30000)}: no DOM mutation for quietMs, no running animation, two frames painted. Returns {revision, settled, waitedMs}; settled false means the timeout came first.

### `POST /ui/setup`

Read the Mistral key screens. No body. READ-ONLY: returns whether a key service exists, whether the setup screen covers the content, the step (create, pickVault, unlock, key, ready, blocked), the vault and key states, the last test outcome and failure reason word, and counts of saved keys and Vibe's own key places. Never a key or a password; changes go through the `keys` command.

The `keys.readStatus` command reports `availableVaults`, `vaultSelected` and `vaultPath` (null before
selection). Vault kinds are `vibeWork` (private app-data vault), `shellx` and `desktop`.

<!-- vocabulary:vault:start -->
The window names them **Vibe Work vault**, **ShellX Vault** and **Vault app**. Its per-project entry
permissions are **Ask each time**, **Allowed** and **Never** (`askEachTime`, `allowed`, `never` on the wire).
**Pause agent access to secrets** pauses agents' use of secrets while sessions keep working;
**Lock vault** protects the vault cryptographically and pauses model requests until unlock.
<!-- vocabulary:vault:end -->

These fields contain only kind and folder metadata. Vault management commands remain user-only.

Vault entry listings include `defaultPermission`, explicit `projectPermissions`, and
`projectPermission` for the chosen permission project (null means inheritance).
The effective `permission` uses that project's setting or the entry default, with
the captured project candidates' Never veto. Older entries default to Ask each time.
Changing a default cannot expand a running session's captured default authority.
`secrets.setDefaultPermission` and `secrets.resetEntryPermission`, like the project
permission setter, are user-only and return `403 vaultUserOnly` to the Debug API.
Replacing or generating a replacement value also requires native confirmation.

### `POST /ui/state`

Read the shell state: theme, list visibility, selected session, panel, the overlay that covers the content (settings, palette, ...), the toasts on screen, the app version (appVersion, as Settings > About shows it; null before the core answered) and the session rows with their status badges.

### `POST /ui/terminal`

The Terminal tab of the shown session. Body: {action: state|selection|selectAll|focus|open|select, terminalId?}: state = its terminal tabs, the active one, whether a terminal has keyboard focus, font size and renderer; selection = the selected text of the active terminal (read only, for "Point Vibe at this"); selectAll selects the whole buffer; focus focuses the active terminal; open opens a new terminal tab (same as the + button); select makes terminalId the active tab.

### `POST /ui/theme`

Set the theme. Body: {theme: dark|light|toggle}. Returns the theme.

### `POST /ui/toasts`

The toasts on screen. Body: {action: read|dismiss, id?}. Returns the list (id, kind, text), including each toast's complete text even when its view shows a preview. Dismissal removes the active toast; the Notifications panel retains its message for this app run (up to 100 recent entries).

### `POST /ui/transcript`

Read or act on the transcript like a click does. Body: {action: read|open|answer|link|sources, sessionId?: string (default: the selected session)}: read returns the rows as rendered (kinds, keys, entry ids, card state, web sources per answer and search step); open {key, open?: bool} opens or closes a group, step or show-all; answer {callbackId, decision?: approve|approve_for_session|approve_permanently|deny|cancel_turn, scope?, feedback?, answers?: [{labels: [string], other?: string}], cancel?: bool} answers a card exactly as its buttons do; link {url} shows the link dialog; sources {key, open?: bool} opens or closes the side list of an answer's web sources.

### `POST /ui/updates`

Read or act on the update flow like the notice and the Settings section do. Body: {action: read|check|download|install|autoDownload|later, enabled?: bool}: read (default) returns whether this build has an update channel, the flow's view (phase: idle|checking|available|downloading|ready|installing|restarting|failed, version, notes, progress, mode inApp|linkOnly, autoDownload, releasePage), the status line and the notice text; check asks the update server now; download and install carry the Debug API origin (logged; no dialog: the request is the user's agreement); install only works for a downloaded and verified (ready) update and restarts the app; autoDownload {enabled} is the Settings switch; later hides the notice.

### `POST /ui/voice`

Read or drive dictation, narration and voice chat like the mic button, the narration bar and the Voice chat button do. Body: {action: read|start|stop|cancel|narrateLast|stopNarration|skipNarration|settings|voiceChat|voiceChatEnd, sessionId?: string (default: the selected session)}: read returns the dictation state (state, streamId, mode, seconds, live text, last inserted text, error, micPending, permissionWait) and narration (enabled, playing, queue, last played: chars, bytes, seconds); start turns the microphone on and answers once it records, failed, or waits for the microphone permission (micPending; an OS prompt has no time limit, poll read); stop inserts the text at the caret of the session's text box (marked as text from an external client); narrateLast narrates the session's last finished turn; settings {narration?, voice?, language?, micId?, voiceChatAutoEnd?, voiceChatPauseMs?} changes the Settings screen's voice choices. Voice chat: voiceChat {on: true|false} turns it on for the session (moving it from another session) or off; voiceChatEnd ends the user's turn now, like pressing the button while it listens; read also returns voiceChat (on, owner, phase: off|listening|sending|thinking|waiting|speaking|error, micPending, live text, what was heard, endedBy: button|pause|limit, the last spoken text with its length and seconds, the spoken count, note, error, autoEnd, pauseMs).

### `POST /ui/worktrees`

Read the worktree UI. Body: {action: state|folder, path?}: state = the new-chat view's folder (newFolder, newFolderBy), the open end-of-session choice (session, why, the inspection: files a removal discards, branch plan, why it is blocked) and `orphans`, the same inventory as the core `worktrees` op `orphans`: leftover copies (deleted session, undecided) and kept copies (`kept: true`), excluding removed/missing copies and copies used by live sessions; state refreshes the inventory; folder {path} chooses a folder for the new-chat view like Choose folder does (pickFolder: it counts as picked for this run). Start, keep and remove go through POST /commands with {type: worktrees} (no native dialog for the Debug API). A `.worktreeinclude` file at the repository root copies the git-ignored files it names into the new copy: `options` returns what would be copied (`include`, names and sizes only) and `start` returns what was copied and what was skipped and why (`include`); file contents are never returned. `options.existingBranch` names the branch when the chosen folder is already a Vibe Work copy; the new-chat switch defaults off in that folder. Saved copies can be chosen from the welcome list with New chat here; the sessions footer Copies count opens that same list.

### `POST /ui/ui-client/poll` and `POST /ui/ui-client/reply`

The transport a UI running in a browser (the developer UI) uses instead of the window's own bridge.

## Evidence workflow

1. `POST /ui/revision` (note `revision`), act (a command or a UI route).
2. `POST /ui/settle` with `{"quietMs": 300}`: it answers when the window stopped changing.
3. `GET /screenshot` (and `POST /ui/layout` for automatic checks: menus inside the window, no cut-off text, no
   horizontal scroll).
