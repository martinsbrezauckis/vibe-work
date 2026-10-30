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

**Only the vault waits for you.** The Debug API acts as you: turning it on is your consent, so it answers
the confirmations the window would ask you itself, and nothing but vault actions waits for you in the app.

**The vault is yours.** On the vault, a Debug API client has exactly the rights the agent has in Vibe Work, nothing
more. It can read what the agent can read: whether a vault exists and is unlocked, whether a key is set, the names of
the secrets agents may use, the website logins (name, site, user name), the "always" grants and the use log (never a
value or a password). Everything else is yours alone, in
the window, by hand:

- managing the vault: creating, unlocking or locking it, its password, the "Lock the vault when Vibe Work quits"
  switch (whether its key stays on this computer across a quit; `keys.readStatus` shows it as `lockVaultOnQuit`);
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
changed on the way (reversed or hex-dumped in a terminal, for example) is not recognised. Chat titles and previews
that hold part of a vault value read `[secret withheld]` in every answer once the vault has been unlocked in this
run.

## Turning it on

- **Settings > Developer > Debug API.** The switch says what it does: "Lets a local agent do what you can do in Vibe Work, terminal and files included, and answers the app's confirmations for you; only your vault stays yours. It hides known secrets where it can, but that is no guarantee." (On the vault it has the agent's rights only, above.) While it runs, the
  status bar says "Debug API" (developer builds also show a badge in the title bar).
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

- The Debug API acts as you. Turning it on is your consent: it answers the confirmations the window would ask you
  itself, <!-- vocabulary:fullAccess:start -->Full access<!-- vocabulary:fullAccess:end --> switches, proxy and certificate changes and the browser's provider permissions included. Only
  vault actions wait for you in the app (the vault dialogs described above). Actions are still labelled: logs, refusal
  events and the transcript say when the Debug API acted, and text a Debug API client typed is marked in the composer.
- No answer is shortened for the Debug API: paths, names, branch lists and inspections are what the window shows.
- The Debug API picks folders like **Choose folder** does (`pickFolder`).
- Rules that are not about you stay: the agent in a session is still limited to its session, pages and previews
  stay sandboxed, methods that are refused for everyone stay refused, and known secret values are hidden in
  answers where Vibe Work can (see the top of this page).
- `ui/*` routes run in the window, and the window holds no Debug API standing: nothing a page sends can claim it.
  While a `ui/*` request that may act is being served, the window's commands are recorded as the Debug API's and
  held to its vault rule. The owner-only effects a route promises (a folder or a destination given as a path, a
  confirmation the route answers as yours) are granted by Vibe Work to exactly the command that request names, once,
  while it is pending. A generic `ui/click`, `ui/key`, `ui/menu` `choose` or palette `run` gets no such grant: a
  native confirmation it leads to opens as the window's; answer it with `POST /asks/{id}`.

## Routes

### `GET /health`

The API version, app and Vibe versions, uptime, why the API is on. Fields: `ok`, `apiVersion`, `appVersion`,
`vibeVersion`, `uptimeMs`, `enabledReason`, `debugApi`, and `rateLimits`: the last per-minute rate-limit headroom a
Mistral answer reported for your key (`tokensLimit`, `tokensRemaining`, `requestsLimit`, `requestsRemaining`,
`observedAtMs`; `null` before the first answer or without a key service). The numbers are per minute: about 60 s after `observedAtMs`
without a newer answer they are history, not the current headroom.
`keepAwake` reports `{enabled, status}` (`idle`, `active`, `refused`); `POST /commands` with
`{"type":"keepAwake","request":{"op":"read"}}` reads the same view, and `{"type":"keepAwake","request":{"op":"write","enabled":false}}`
saves the setting and releases an active hold before returning. It defaults on; only working chats inhibit idle
sleep, including subagents. Approval/question waits, completion, Stop, failure and exit release it; screen sleep,
locking and explicit Sleep remain available. The `keepAwake` event publishes changes to this view.

```sh
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/health
```

### `GET /server`

The Vibe app-server's state (running, restarting, `notInstalled` before Vibe is installed at setup, its last stderr lines with secrets replaced). While Vibe is not installed, every command that needs it answers 503 `serverUnavailable` with the reason word `notInstalled` at once.

### `GET /sessions`

The session list (`?limit=`, `?cursor=`, `?includeArchived=true`).

### `GET /sessions/{id}`

One session as the app holds it (state, history, turns). An open file edit or write card carries its preview as
`detail.effect.vwPreview` (`lines` with `kind`, `oldNo`, `newNo`, `text`, `cut`; `more`, `cutChars`, `added`,
`removed`, `newFile`), made from the file when the card was first seen; none for a file outside the chat's
folders. `ask.edit-preview-whole` in `ui/controls` is the card's Show the whole change.

### `GET /sessions/{id}/anomalies`

The session's protocol anomaly counters.

### `GET /sessions/{id}/processes`

The session's processes and listening ports (owner, evidence, leases).

### `GET /sessions/{id}/changes`

The Changes tab's view of the session (diff against its review baseline, or its turns' file edits). A Markdown folder link in an agent handover, such as `[Changes](git%20test/)`, opens this tab with that folder selected only when Git status discovered the folder as a repository.

### `GET /ui/trace`

Reads the last 500 commands the window sent to the core. It does not perform an action or return command results.
Use `?since=<sequence number>` to receive only newer entries; without it, the cursor is zero.

The answer has `entries`, `latestSeq`, `oldestSeq` (null when empty), and `truncated`. Each entry has:

| Field | Meaning |
|---|---|
| `seq` | Increasing sequence number within this window |
| `time` | Dispatch time in UTC |
| `type` | Command type |
| `op`, `method` | The request's operation or method, when present |
| `summary` | At most 200 characters of allowlisted ids and names |
| `testId` | The triggering control's stable `data-testid`, or null when unknown |
| `outcome` | `status` is `pending`, `succeeded` or `failed`; a failure also has its error `code` |

The trace stores metadata only. It never stores file contents, messages, passwords, keys, tokens or raw request
bodies or raw error text. A command appears as pending when sent. The same entry's outcome changes when
that call completes, keeping its sequence and triggering hook. A refused command is failed, with only its error
code. Read again with the cursor saved before the click to see updated outcomes; presence alone does not prove
success.
Host actions such as opening a native picker are outside the typed command trace.

Save `latestSeq` before calling `POST /ui/click`, then read with that cursor to see the commands the click sent.
A direct click is attributed during its dispatch; a Debug API click also keeps its attribution through pending
work until its UI request finishes. Later unrelated work can have a null control id. No control id is guessed
from visible or translated text. Repeated list controls carry the item in `data-item-id`; the trace records the
control id and only allowlisted command metadata.

`truncated: true` means entries newer than your cursor have already been discarded. Read again with a cursor one less than
`oldestSeq` to include the oldest retained entry. The buffer is in memory, starts empty with a new window, and resets
when that window reloads. An invalid or negative cursor answers 400. A missing UI handler answers 404; a window
that does not answer uses the existing UI request timeout. The route has the same authentication and secret
redaction as the other Debug API routes.

### `GET /events`

Server-Sent Events: snapshots, then ordered changes and app events. `?session=<id>` for one session. Event
names:

| Event | What happened |
|---|---|
| `snapshot` | the full state of one session (replaces your copy) |
| `change` | an incremental change after the snapshot it follows |
| `lagged` | you fell behind and missed items; fresh snapshots follow |
| `server` | the Vibe app-server's state changed |
| `guardRefused` | a guard stopped an action; includes the session, guard kind, safe explanation, optional step id, time and refusal id for the notification history. Protected-folder declines use their own guard kind. |
| `protectedChange` | a protected-folder review changed; `proposal` contains its session, literal diffs, status and applied paths. Pending captures live only in memory; a review still pending at a restart is saved as stopped (`stopped: true`) and cannot be applied. |
| `callbackOpened` | Vibe asks for an answer (approve, question); answer with `answerCallback` |
| `callbackAnswered` | a callback was answered, and by whom |
| `refused` | a command was refused: a gate or policy refusal, never a plain `notFound` answer (never with its parameter values) |
| `alert` | something the user must see at once |
| `sessionStuck` | a session could not be reopened after a restart (`retryReopen`, or continue in a new one) |
| `sessionHolder` | another process (the Vibe CLI, another app) holds a session a window shows (`holderPid`), or let it go (`null`): read-only here meanwhile; checked every 2 s |
| `queueReturned` | messages that waited in a session's queue when its turn was stopped or failed were taken out of Vibe's queue, never sent (`items`: Vibe's queued items, oldest first); the window puts them back into the composer |
| `debugApi` | the Debug API was switched on or off |
| `keepAwake` | the app-wide idle-sleep preference or hold status changed (`enabled`, `status`) |
| `panelRead` | a session's agent called `panel_read` (`requestId`, `sessionId`): the window answers with `files` `panelAnswer` (what that session's side panel shows: tabs, open editors with unsaved text, selection and cursor line, the Files tab) within 5 s, and its editors show "Vibe is reading this editor" meanwhile |
| `githubReads` | the GitHub reader opt-in or account status changed (`enabled`, `status`, `login`, `reason`); never credentials |
| `askOpened` | a permission prompt opened (`ask`: id, kind, session, title, deadline, offers, `personOnly`); never request parameters |
| `askClosed` | a permission prompt closed with one outcome (`record`: the ask's metadata, `closedAt`, `outcome`); never request parameters |
| `chromeAsk` | a Chrome Control card opened, or a call the user set to Never was refused (`entry`: `askId`, `sessionId`, `turnId`, `createdAtMs`, `tool`, `summary`, `group`, `reason`, `covers`, the call's `input` as Vibe sent it, `open`); the card's outcome is the `askClosed` record with the same id |
| `chromeControl` | Settings > Browser > Your Chrome changed (rows saved, set up, removed): the whole `chromeControl` view (level, setup status, browsers, profiles, tools) |
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
| `secretScan` | session-log scan settings, state or findings changed: the whole scan view, with secret names, login ids and counts; never secret values or log contents |
| `secretsChanged` | project secrets, their grants or the use log changed: read them with `secrets` `list`; never a value |
| `pagesChanged` | a session's Project tab changed: read it with `pages` |
| `pageMenu` | a person right-clicked the Project tab's page: `{sessionId, x, y, selected}` (the point in the page area, whether text was selected; no page text); the window opens its page menu there |
| `filesChanged` | a session's File tab viewer changed: read it with `files` `viewer` |
| `lineCommentsChanged` | a session's diff comments changed: read them with `lineComments` |
| `projectsChanged` | your project groups changed: read them with `projects` `list` |
| `worktreesChanged` | a session copy was created or removed: read the count with `worktrees` `count` |
| `gitBrowserChanged` | a watched saved project's files or Git metadata changed: read `gitBrowser` for that `projectId` |
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

### `GET /asks`, `POST /asks/{id}` and `POST /asks/grants/{id}/remove`

`GET /asks` returns `{open, records, grants}`: pending permissions, the newest 500 outcomes from this app run, and
the session permissions Vibe Work keeps for Vibe's approvals ("Allow for this session").
Each ask has an id, kind, optional session, plain title, deadline (`null`: it waits until answered), offered answers and
`personOnly`. Records contain metadata and the outcome; they contain no request parameters or vault values and
are kept in memory.

Answer with `{"answer":"once"}`, `{"answer":"session"}` when offered, or `{"answer":"no","note":"plain feedback"}`
(the note stays in the ask's record; the agent gets only the result below).
A person-only vault ask returns 403 and must be answered in its native dialog. Late answers are refused.
Every allow reaches Vibe as a single allow; "session" is kept by Vibe Work for that session, and a later call of the
same tool with exactly the same permissions is allowed without a card (its record says `by: "grant"`). An ask with
`everyCall` covers every call of its tool in that session. Vibe's "Always allow" is not offered. Posting to
`/asks/grants/{id}/remove` removes one kept session permission (the next matching call asks again).
These routes are available only while the Debug API is on.

A permission card waits until it is answered; it has no expiry. A No on Vibe's own approval card reaches Vibe as
`cancel_turn` and stops the turn (for a subagent's card, the whole tree); a No on the parent's card also stops the
subagents its turn started. Vibe Work's own prompts (Git guard, vault, documents, processes, protected
changes, and the cards for Vibe Work's tools) hold the tool call until they are answered: Allow runs that call once
in the same turn, a No returns `Not run: the user declined this. Do not do it another way. Tell the user why it is
needed and wait for their reply before you go on with it.` as the tool's result and the turn goes on (a call held
this way inside `run_typescript`, such as a card for Vibe Work's tools, reaches the program as Vibe's own
`Tool execution was skipped by Runtime policy.`), and Stop closes the prompt without running it. `secret_run` returns the command's result in the same call,
or its request id when it runs longer; read `secret_run_status` in a new call. Ctrl/Cmd+Shift+A goes to a waiting
prompt. A terminal `answerNotConfirmed` outcome means Vibe did not confirm delivery of the attempted answer; it is not
an allowed or refused action.

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

The secret-field check runs even while another UI action is pending and waits for its final geometry, including
when painting takes longer than the UI's acknowledgement budget. It waits up to two seconds for a loaded viewport
and the locked fields to paint. A refusal names the reason: `the page has not loaded a viewport`,
`the page did not paint the secret fields in time`, or the UI's timeout or error. An unexpected `busy` or `pending`
answer is still refused, with `the window is busy with another request` or
`the UI has not finished checking secret fields`; a malformed answer can say `the UI returned no viewport width`.

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

Test builds support command checks against the pinned real app server with scripted model and service responses.
Checks of UI and local core state can run without a Vibe process. These checks require no model account; they do not
validate real-model answer quality.

## Commands (`POST /commands`)

Every command is `{"type": "<name>", ...}`. Requests with an `op` take `{"type": "<name>", "request": {"op": ...}}`.

| Type | What it does | Example |
|---|---|---|
| `listSessions` | the session list | `{"type":"listSessions","limit":20}` |
| `readSession` | one session | `{"type":"readSession","sessionId":"<id>"}` |
| `refreshPlugins` | re-pins plugins and reopens with host guards; working or never-run chats wait for turn end | `{ "type":"refreshPlugins","sessionId":"<id>" }` |
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
| `checkDefaultModel` | the launch check the window runs once Vibe is running: a default Vibe no longer offers is named (`gone`, else `null`), and the window then asks for a new one (`ui/models`); the default and every model entry stay as they are (Mistral's own Large 4 entry, when listed, only gains the window and prices it does not report) | `{"type":"checkDefaultModel"}` |
| `setAccessMode` | the access mode, <!-- vocabulary:fullAccess:start -->Full access<!-- vocabulary:fullAccess:end --> (`auto-approve`) included: the Debug API answers its own confirmation | `{"type":"setAccessMode","sessionId":"<id>","agentName":"accept-edits"}` |
| `answerCallback` | answers an approval or question | `{"type":"answerCallback","sessionId":"<id>","callbackId":"<cb>","output":{"type":"approval","decision":{"type":"approve"}}}` |
| `decideProtectedChange` | applies one pending review in its live session, or with `approve:false` writes nothing and answers the proposal like a No in Vibe Work's other prompts (the turn goes on); only the window or Debug API may decide. Returns the final review, including any applied prefix on failure. | `{"type":"decideProtectedChange","sessionId":"<id>","proposalId":"<id>","approve":true}` |
| `listCallbacks` | the open callbacks | `{"type":"listCallbacks"}` |
| `requestConfirmation` | a single-use token for one exact call; the Debug API gets it at once (only vault dialogs wait for you) | `{"type":"requestConfirmation","method":"session/delete","params":{"sessionId":"<id>"}}` |
| `call` | any other Vibe method the app allows; `tools/list` (which Vibe's unified harness does not have) is answered by Vibe Work from the session's `runtime/read`: Vibe's own tools, then each MCP server's enabled tools as `<server>.<tool>`. Settings' Vibe values are `config/write` (`sessionId`, `ops`): `{"op":"set","path":"/models/<alias>/auto_compact_threshold","value":200000}` sets that model's Compaction point (a whole token count from 10,000; Vibe refuses one its window does not allow), `{"op":"remove","path":...}` gives back Default | `{"type":"call","method":"session/delete","params":{"sessionId":"<id>"}}` |
| `viewSession` | which session this client shows | `{"type":"viewSession","sessionId":"<id>"}` |
| `retryReopen` | tries a stuck session again | `{"type":"retryReopen","sessionId":"<id>"}` |
| `cancelReopen` | stops that try | `{"type":"cancelReopen","sessionId":"<id>"}` |
| `continueInNewSession` | continues a stuck session in a new one | `{"type":"continueInNewSession","sessionId":"<id>"}` |
| `readStatus` | app status, versions, Debug API flag | `{"type":"readStatus"}` |
| `readAnomalies` | anomaly counters | `{"type":"readAnomalies","sessionId":"<id>"}` |
| `vibe` | installing Vibe at setup: `readStatus`, `prepare` (an offer: sources, checksums, the commands that will run, sizes, an `offerId`), `confirm` (`offerId`: the user's consent to exactly that offer; starts the install), `cancel`, `checkExisting` (finds Vibe installs already on the computer and reads their versions), `useExisting` (`path`, `acceptWarning` for another version). Vibe updates: `prepareUpdate` (the offer for the newer tested Vibe the last update check found in the signed supported-Vibe list: sources with checksums, the commands, sizes, an `offerId`), `confirmUpdate` (`offerId`: installs it next to the running Vibe; it starts with Vibe Work's next start), `cancelUpdate`; the view's `update` part says what the last check found (`available`, `upcoming`: newer on PyPI but not supported yet, never installed, `mismatch`: a notice for your own Vibe install, `checkError`). The check itself runs only with the app update check (`POST /ui/updates` `check`). Answers the install view; refusals are `vibeInstall` with a reason word (`wrongState`, `staleOffer`, `unsupported`, `unknownExisting`, `needsAcceptance`, `disk`, `noUpdate`, `notConfigured`) | `{"type":"vibe","request":{"op":"prepare"}}` |
| `keys` | the Mistral key and the vault: `readStatus`, `listKeyCandidates`, `testKey`, `detectVibeKey` (never answers a key). `createVault`, `unlockVault`, `lockVault`, `setLockVaultOnQuit`, `setupStoreKey`, `removeKey`, `useExistingKey`, `importVibeKey`, `removeVibeKeyOriginal` are the window's: refused here (`403 vaultUserOnly`, "The vault is yours" above) | `{"type":"keys","request":{"op":"readStatus"}}` |
| `terminal` | terminals: `open`, `list`, `write` (sends input with your rights), `resize`, `read`, `restart`, `close` | `{"type":"terminal","request":{"op":"list"}}` |
| `newChat` | the New chat screen before the first message: `state` (its `ownerId`, folder and home folder; use `ownerId` as the `sessionId` of `terminal`, `preview` and `browser` requests there), `setFolder` (`folder`, or none for the home folder: where new terminals open), `adopt` (`sessionId`: hands what is open to a started session, App preview sign-ins included, nothing stops, and gives the screen a new `ownerId`), `closeAll` (stops its terminals, closes its pages, clears their local site data and gives the screen a new `ownerId`). Never reaches Vibe | `{"type":"newChat","request":{"op":"state"}}` |
| `instructions` | the AGENTS.md files: `read`, `readCli` (reads the command-line copy), `write` (replaces the app or project file, comparing optional `expected` text to prevent stale edits), `apply` (reopens affected sessions so they take current instructions). These commands currently run with Debug API user rights. | `{"type":"instructions","request":{"op":"read","scope":"personal"}}` |
| `documents` | documents in the composer: `list`, `read`, `save` (`id`, `sessionId`: saves original bytes under the project attachments folder and returns an absolute `path`), `extract` (automatic OCR on attach, or retry), `release` (pending bytes after sending), `remove`, `takePicture`, `addDropped` (files dropped on a composer, base64) | `{"type":"documents","request":{"op":"list"}}` |
| `files` | the File tab: `resolve` (paths as the chat wrote them, in the session folder or a picked folder; a relative path is tried in the chat's own folder first), `read`, `stat`, `view`, `layout`, `reload`, `viewer`, `clearExternal`, `screenshot`, `open` (default app; never a program), `reveal`, `write` (a quick edit: the whole new text and the `fingerprint` it started from; `stale` when the file changed on disk), `panelAnswer` (the window's answer to a `panelRead` event: `sessionId`, `requestId` and the typed `panel`; taken only for the request's own session, once) | `{"type":"files","request":{"op":"resolve","sessionId":"<id>","paths":["src/main.rs"]}}` |
| `speech` | dictation and speech: `start`, `append`, `stop`, `cancel`, `speak`, `voices`; `readAloud` reads one reply in the open window without changing voice settings | `{"type":"speech","request":{"op":"voices"}}` |
| `changes` | the Changes tab: `read`, `keep`, `revert`, `restart`, `previewUndo`, `folderSessions`, `planHistory`, `enableHistory` (`confirmed: true`), `dismissOffer`, `readRepo` (`sessionId`, `repo`: a nested repository's uncommitted files against its last commit, read only; `.` answers like `read`) | `{"type":"changes","request":{"op":"read","sessionId":"<id>"}}` |
| `lineComments` | comments on diff lines: `list`, `add`, `remove`, `markSent` | `{"type":"lineComments","request":{"op":"list","sessionId":"<id>"}}` |
| `preview` | the pinned App preview: `list`, `open` (this session's local servers only), `back`, `forward`, `reload`, `close`, `setWidth`, `layout`, `browserOverlay`, `screenshot`, `console`, `explain`, `pick` (the Debug API names `target`: `{selector}` or `{x, y}`; with `tabId`, a web page of the session's Browser tab, only its selection is read, never a click; the answer's `url` is the address the engine loaded, empty when the page was loading or changed during the pick, and `text` is the rendered text), `cancelPick`, `stopOwner` | `{"type":"preview","request":{"op":"list","sessionId":"<id>"}}` |
| `browser` | the agent browser of a session: `read` (the page as the agent reads it: refs, a page of text, prompt-guard flags; `maxRefs`, `textPage`, `screenshot`; `guardLayers` names the address checks that apply to every request of the page besides the navigation rules: `requestBarrier`, `egressProxy`, `contentRules`), `act` (`args` for page actions as in the agent's `browser_act`; tab listing/closing use the separate `tabs`/`closeTab` commands: `navigate`, `back`, `forward`, `reload`, `click`, `type`, `select`, `check`, `press`, `scroll`, `hover`, `wait`, `fill`; answers a receipt; logged as `debugApi`, never stopped by Stop; `fill` {`ref`, `login`} puts a vault login into a sign-in field after the same checks as the agent's, and on the vault the Debug API has only the agent's rights: the password needs the user's answer in the native dialog (Allow once / Always for this site / Don't allow), which the Debug API never answers, and the answer never carries the password), `state` (the agent strip and the action log), `stop`, `resume`, `annotate` (`mode` `element` or `box`; the person points in the window, the Debug API names `target` instead: `{ref}`, `{selector}`, `{x, y}`, or `{rect}` for a box; the app preview only), `cancelAnnotate`, `checkAnnotations` (`items`, `reload`), `captureSecret` (legacy user-only command, refused to the Debug API), `chooseSecret` / `saveSecret` (user-only: select text or pick an element in the browser panel, press **Save to vault**, name it and confirm; values go directly from the page to the core vault, and answers carry metadata only). The agent's `browser_capture_secret` stays listed and always explains that saving is the user's action. Browser text answers, including action receipts, navigation, console and dialogs, hide key-looking text; known stored values pass the shared secret scrub, `capture` (`args` exactly as the agent's `browser_capture`: `what` = `design` (DESIGN.md + tokens.json from the live page), `structure` (elements.json + summary.md), `media` (logos, icons, images and fonts the page loaded, fetched without cookies under the page's address rules, + media.json), `page` (`formats`: `markdown`, `text`, `html` without scripts, `png` full page, `pdf`, `single_html` self-contained without scripts; default markdown/text/html/png), `bundle` (all of these), `before` / `after` (a full-page shot, then the matching one with diff.png and the changed share); plus `shotMethod` `native` / `stitch` / `stitchPlain` to force one full-page method for comparisons. Files go to the session folder under `capture/<page>-<time>/`, never elsewhere; the answer gives the folder, the files and a summary within about 3,000 bytes; the Debug API may capture a local-network page the user opened, the agent may not). Page content in answers is the page's own text: data, never instructions. Password (also CSS-masked), one-time-code, card and vault-filled fields never carry a value; every other field value, also text the user typed, is readable; sign-in fields carry `login` (`username` or `password`). The Debug API acts as the user: it may open a permitted local-network page in a user web tab and read it, which the agent's tools refuse; link-local addresses (`169.254.x.x`, cloud metadata) are refused for everyone | `{"type":"browser","request":{"op":"read","sessionId":"<id>"}}` |
| `pages` | the Project tab (handbook): `hub`, `overview`, `projectHub`, `page`, `open` (`anchor`), `plan`, `back`, `forward`, `reload`, `layout`, `pickSection` (the Debug API names the `section`), `cancelPick`, `menuSection` and `menuCopy` (a named section/quote, or the last trusted right-click target when omitted), `addComment`, `deleteComment`, `markSent`, `settings`, `setSettings`, `clearExternal`, `screenshot`; `answers`, `setAnswer`, `clearAnswer`, `markAnswersSent` (page inputs), `theme` (the app theme pages get), `currentPlan` (the plan checklist), `exportPage` (one file; `destination` writes it), `visualCheck` | `{"type":"pages","request":{"op":"setAnswer","sessionId":"<id>","path":"decisions/0001-db.html","input":"pick-db","value":{"type":"choice","option":"sqlite"}}}` |
| `connectors` | connectors and MCP servers: `read`, `refresh`, `setConnectorOff`, `signIn`, `connect` (`sessionId`, `alias`: a connector that needs no sign-in; Vibe makes the default connection and the open sessions refresh their connectors), `cancelSignIn`, `addServer`, `signInServer`, `signOutServer`, `setServerOff`, `removeServer`, `addKeyServer` (`url`, `name?`, `secretId`, `header?`: a server that needs an API key; the key stays in the vault and Vibe Work adds it to each request; the user decides in the native dialog; `newSecret {name, value}` is the window's: refused here, `403 vaultUserOnly`), `allowServerKey` (`name`; the native dialog decides). `removeServer` of a server that uses a key from the vault is refused here (`403 vaultUserOnly`) | `{"type":"connectors","request":{"op":"read"}}` |
| `marketplace` | the Tools marketplace (reviewed, pinned catalog): `read`, `offerInstall` (`id`: what an install puts on this computer, as a single-use offer: name, publisher, source, licence, what runs), `install` (also updates; optional `offerId`: from the window a shown offer is the consent and no native dialog opens; runs in the background, follow `marketInstall` events), `cancelInstall`, `remove`, `turnOnPlugin`, `turnOffPlugin`, `removePlugin` (each takes a `path` from `read`'s `discovered` list, and optional `sessionId` or `cwd` for the opened project; only plugins in Vibe Work's own home or that project; the answer is the view after the change; Turn on and Remove ask in a native dialog, answered for the Debug API like every other confirmation) | `{"type":"marketplace","request":{"op":"install","id":"context7"}}` |
| `skills` | Tools skills manager in the selected chat or work folder: `read`, `detail` (`name`), `setEnabled` (`name`, `enabled`), `offerImport` (`skillId`) and `offerUpdate` (`name`): what Install or Update would add, as a single-use offer; `import` (`skillId`, optional `offerId`), `setLatest` (`name`, optional `offerId`): from the window a shown offer is the consent and no native dialog opens, a used or old offer is refused (`staleOffer`); `remove` (`name`). Set `sessionId` or a natively selected `cwd` in the request. Uses a temporary idle helper chat; changes are refused while chats in that folder are busy and apply to new chats. Returns a view or skill detail | `{"type":"skills","request":{"op":"read","sessionId":"s1"}}` |
| `libraries` | your Mistral document libraries: `list` (with scopes and use facts), `create` (`name`, optional `description`, and a `scope` object such as `{"kind":"off"}`, `{"kind":"global"}` or `{"kind":"project","projectId":"<id>"}`), `setScope` (`libraryId`, `scope`), `delete` (on Mistral, with its documents), `documents`, `upload` (files as base64; they go to Mistral), `deleteDocument`, `text` (the extracted text a source opens), `ask` (`question`, `libraryIds`, with an empty list meaning all enabled libraries) | `{"type":"libraries","request":{"op":"list"}}` |
| `githubReads` | app-wide GitHub reads with the user's gh login: `read` refreshes availability; `write` (`enabled`) saves the opt-in and starts or stops the reader. Returns `view: {enabled, status, login, reason}`; statuses are `missing`, `signedOut`, `signedIn`, `starting`, `on`, `error`, `enterprise`, `loggedOut`. Defaults off, supports github.com only. The token goes from app-side gh to broker stdin and never enters this API or the agent. New chats, subagents and agent terminals use a credential-free gh shim; writes are refused. `gh --version` and `gh help` stay available while off. | `{"type":"githubReads","request":{"op":"write","enabled":true}}` |
| `askGrants` | the session permissions Vibe Work keeps for Vibe's approvals: `list`, `remove` (`id`), or `removeMany` (`ids`, one step; ids already gone are skipped). Returns `grants` (session, tool, permissions, labels, `everyCall`, `by`, `covers` as words and code pieces, `uses`, `lastUsedAt`), `offers` (each open card's `callbackId`, `session` and the `covers` of its "Allow for this session") and `removed`. They end with the session. | `{"type":"askGrants","request":{"op":"list"}}` |
| `chromeControl` | Vibe Work Browser Control (Settings > Browser > Your Chrome): `read` (the `level`: `browse` = Basic browser control, `fill` = Advanced, `act` = Full: works on your behalf; the setup status: `notSetUp`, `waitingForExtension`, `extensionAdded`, the registered browsers, profiles, whether its tools are on, and the owned helper's live `pairing` and public `fingerprint` (cleared on disconnect); a detected extension folder alone does not establish pairing), `setLevel` (`level`), `answer` (`askId`, `answer`: once, session or no, `note`; a Chrome Control card), `offerSetup` (the offer the Set up dialog shows), `setup` (`offerId`: confirms that offer, installs and turns the tools on), `openStore`, `remove`. Raising the level asks a native confirmation for the window; a direct Debug API command or typed `ui/settings chrome level` request answers it as the owner. A generic click gets no grant; answer its non-vault ask through `POST /asks/{id}`. | `{"type":"chromeControl","request":{"op":"read"}}` |
| `vibeRules` | lasting rules earlier "Always allow" answers left in Vibe's settings: `list` (each `{tool, pattern}`; no pattern means every call of the tool; `defaultsKnown` tells whether Vibe's own default patterns were left out), `remove` (`tool`, optional `pattern`), or `removeMany` (`rules`: a list of `{tool, pattern?}`, one settings write; rules already gone are skipped; `removed` says how many went), written through Vibe's own settings. A session that is already open may keep using a removed rule until it ends. | `{"type":"vibeRules","request":{"op":"list"}}` |
| `images` | `read` returns image generation costs, limits and the number of requests made this app run. There is no separate image approval switch or `allow` operation; the agent's image tool follows the normal tool approval flow. | `{"type":"images","request":{"op":"read"}}` |
| `worktrees` | a session in its own copy: `options`, `suggestName`, `start`, `info`, `inspect`, `remove`, `orphans`, `count` | `{"type":"worktrees","request":{"op":"count"}}` |
| `gitBrowser` | bounded saved-project discovery and state: `read`, `watch`, `unwatch`; actions use the existing `git` command through `ui/git-browser` | `{"type":"gitBrowser","request":{"op":"read","projectId":"<id>"}}` |
| `git` | the commit name and email: `identity` (the name and email Vibe Work uses for commits, with `source` `app`, `global` (read from your global git settings, never written there) or `unset`), `setIdentity` (`identity` {`name`, `email`}; saved in Vibe Work's own data; both empty means you are asked when version history is turned on); these two take no `sessionId` or `repo`. The Changes tab's git loop: `status`, `prepareCommit`, `draftMessage`, `commit`, `pushPlan`, `push`, `prPlan`, `createPr`, `prStatus` (`refresh` asks the forge), `prepareMessage` (`checks`, `review`, `updateFromBase`; the text, never sent), `bringPlan`, `bringIn`, `settings`, `setSettings`; `commit` also takes an optional `identity` (the name and email entered in the Commit dialog), saved in that repository's own git settings before the commit when it has no name or email yet. Every request of the git loop accepts `repo`, a repository root relative to the session folder (`.` for the session repository, e.g. `sub` for a nested repository). Status returns `repos` (name, root, branch) and the chosen `repo`; omitted `repo` selects the session repository when present, otherwise the first discovered repository. `push`, `createPr` and `bringIn` show no dialog for the Debug API (your request is the consent) and run only what the plan named | `{"type":"git","request":{"op":"status","sessionId":"<id>","repo":"sub"}}` |
| `projects` | your project groups: `list`, `refresh`, `create`, `rename`, `setFolder` (`folder: null` clears it), `delete` (sessions stay), `move`, `place`, `start` (requires `projectId` and `agentConfig`; starts a session in a project) | `{"type":"projects","request":{"op":"list"}}` |
| `projectInstructions` | Project settings instruction files: `list`, `read`, `linkDraft`, `create`, `write`, `delete`, with `projectId` and an allowlisted relative `name`. Reads include content snapshots; writes/deletes must supply the current snapshot. The Debug API acts as the owner, including deletion confirmation; linked paths and stale snapshots are refused. | `{"type":"projectInstructions","request":{"op":"list","projectId":"<id>"}}` |
| `copyText` | Writes supplied ordinary text to the native clipboard (at most one million bytes) and returns `done` only after the OS accepts it. It never reads clipboard contents or vault values. UI Copy controls use this during a Debug API action because synthetic clicks have no browser clipboard gesture. Ordinary text has no expiry; vault copy keeps its separate person-only command and expiry. | `{"type":"copyText","text":"Text to copy"}` |
| `processes` | `leftRunning` (processes a deleted session left running), `afterDelete` (the window's question after a delete; the Debug API gets the list), `stop` (one listed `id`); `session` lists one active session's processes, `output` reads one process's recent output and `stopSession` stops one, each with `sessionId` and an `id` from that list where needed | `{"type":"processes","request":{"op":"session","sessionId":"<id>"}}` |
| `secrets` | `list` returns secret, login, grant and use metadata without values; `listForProject` also returns each entry's permission for a `project` folder. All other operations, including `setEntryPermission` (`project`, `id`, `permission`), adding, generating, renaming, replacing, copying, removing, revoking and changing agent access, are refused to the Debug API (`403 vaultUserOnly`). Use the window for those choices. | `{"type":"secrets","request":{"op":"listForProject","project":"<folder>"}}` |
| `secretScan` | `read` returns saved `settings` (`enabled`), scan status and retained findings. `write` saves the switch; `autoRedact`, from earlier versions, is refused as an unknown field. `checkNow` requests a full background scan. `redact` takes a `findingId` from that view and redacts its closed chat; past findings remain actionable with scanning off. A finding's `redactable` is true when one of its chat's files is a log from before Vibe 3.0; files of a Vibe 3.0 chat are sealed and left unchanged (the window offers Open chat; deleting the chat removes the value). Nothing is redacted without `redact`. `activity` resets the idle clock. No secret values, file contents or locations enter findings. Password findings include a metadata-only `loginId` so Change password opens that login in Settings; replacing its password remains a person’s Vault action in the window. | `{"type":"secretScan","request":{"op":"read"}}` |
| `safety` | the safety reminders the agent's commands are checked against: `read`, `setRule` (`id`, `on`), `addCustom` (`text`: a command that contains it is stopped), `removeCustom` (`id`). Reminders against mistakes, not a sandbox | `{"type":"safety","request":{"op":"read"}}` |
| `network` | the proxy and certificate settings Vibe keeps, as Settings > Network shows them: `read` (the values Vibe Work keeps in its own file, needing no Vibe; an address with a sign-in comes back as `http://***@host:8080`, never with the sign-in; also what this computer's environment overrides and whether a saved change waits for a restart), `test` (`changes`: one request to Mistral's model list, without a key, with the saved settings and these values laid over them; nothing is saved), `save` (`changes`; writes Vibe's settings and Vibe Work's own file, so it needs Vibe running; a value that fails a check comes back as `refused` with one sentence per field). `changes` holds any of `httpProxy`, `httpsProxy`, `allProxy`, `noProxy`, `certificateFile`, `certificateFolder`: an absent field is unchanged, an empty one removes the setting. A save from the Debug API needs no dialog: it is answered like every other confirmation | `{"type":"network","request":{"op":"read"}}` |
| `pickFolder` | chooses a folder like Choose folder (Debug API only) | `{"type":"pickFolder","path":"<absolute folder>"}` |
| `search` | search inside messages and files (the same results Ctrl K shows): `messages` (every conversation's messages, or one with `sessionId`; the first search fills the index from Vibe's stored history, `index.filling` says it is still running), `files` (names and lines in the sessions' folders, or one session's; ignored and build folders skipped), `status` | `{"type":"search","request":{"op":"messages","query":"rate limit"}}` |
| `sessionTools` | a session as Markdown and temporary chats: `export` (the whole session as Markdown; tool steps summarised, file changes as diffs, known secret values replaced; `destination`, an absolute path, writes the file, else the text comes back), `startTemporary` (a chat Vibe Work deletes when it quits), `temporary` (their ids), `holder` (the pid of another process holding a session now, such as the Vibe CLI or another Vibe Work window; `null` when it is free or held by this app), `setSubagents` (`enabled`: saves the Subagents setting for new chats through a short-lived empty chat that is deleted again; chats that are already open keep their value, and the answer's `leftoverSessionId` names that chat if it could not be deleted), `excerpt` (`sessionId`: `{title, firstUser, lastAnswer}`, the session's title, first message and last answer, each bounded, about 2 KB, vault values withheld: what a session mention in the composer carries; reads the session without taking it) | `{"type":"sessionTools","request":{"op":"export","sessionId":"<id>","destination":"<absolute path>.md"}}` |

Project entry permissions are `askEachTime`, `allowed` and `never`. `secrets.listForProject` accepts an existing
project folder, and the user sets an individual secret or login's choice in the window with `setEntryPermission`.
A Debug API client may read the choice but cannot change it. See the [vocabulary reference](VOCABULARY.md) for
the displayed names.

The `browser` command also has user-only Settings account controls: `accounts` returns provider sign-in hints
without cookie values; `signIn` and `signOut` take a `provider`; `accountsLayout` takes an optional `rect` to
place or hide the sign-in page; `closeSignIn` closes that page without clearing its provider sessions. The
sign-in page is reserved for direct user input, and normal browser `read` or `act` requests cannot target it.
For a focused browser read, `read` accepts `find` and `maxBytes` as well as `maxRefs`, `textPage` and `screenshot`.

Picks never wait for a person. A pick in the window is a person's click; over the Debug API there is no pointer, so
the request names what a person would have clicked and the answer comes at once with the same shape:
`browser.annotate` takes `target` (`{ref}` from a read of the page, `{selector}`, `{x, y}` in viewport pixels, or
`{rect: {x, y, width, height}}` with `mode: box`), `preview.pick` takes `target` (`{selector}` or `{x, y}`) and
`pages.pickSection` takes `section` (an id the shown page file has). A request without its target answers
`400 invalidParams` naming the field, before the session is read. `ui/browser` → `annotate` and `ui/project` →
`pick` and `point` take the same fields.

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

Use `POST /ui/controls` to discover rendered controls (`data-testid` and repeated `data-item-id`), then
`POST /ui/click` or `POST /ui/key` to act. Read the feature's state and the actual command result afterwards.
`GET /ui/trace` reports pending/succeeded/failed commands; a sent command alone is not a finished result.
Controls generated inside native pages or CodeMirror use the typed actions below instead of synthetic clicks.

### `POST /ui/browser`

The Browser tab's agent browser: the agent strip (agent active, acting, stopped), the action log, the address bar, the notes on the app preview and the Capture menu, every action the tab has. Body: {action: state|tabs|openTab|switchTab|closeTab|pointAt|copy|saveToVault|stop|resume|annotate|cancel|note|remove|send|recheck|address|capture, sessionId?: string (default: the selected session), ...}: state = the open URL, the ordered tabs and selected tab, whether notes are offered (pages of the session's own local servers only), the strip, the log (who: agent|debugApi|user, action, target, outcome, detail), the notes (id, mode, role, name, route, note, status draft|sent|changed|unchanged|missing, source hint, whether a crop exists, by ui|debug-api: a note a Debug API request created or edited stays `debug-api`), a running pick and the last error; tabs refreshes the tab list; openTab {url?} creates a user-controlled web tab; switchTab {tabId} selects an owned tab; closeTab {tabId} closes a web tab (the pinned App preview is refused); stop / resume = the strip's buttons (Stop cancels the agent's browser action in flight and refuses more until Resume or the session's next message); annotate {mode: element|box, target} annotates the element or area `target` names and answers the new note at once: you have no pointer, so name it with `{ref}` (from a read of the page), `{selector}`, `{x, y}` (viewport CSS pixels), or in box mode `{rect: {x, y, width, height}}`; without `target` the answer is `400 invalidParams` naming the field; cancel stops a pick; note {id, text}; remove {id}; send puts every draft note into the composer (a context chip and a cropped picture each, never a command; marked as the Debug API's when the send or the note came from it, also when the user later sends such a note from the window) and answers how many; recheck {reload?: bool} reloads the preview and sets each sent note to changed / unchanged / missing; address {url} opens a URL in the selected web tab with the user's navigation rights; from App preview it opens a local server there or creates a web tab for a public URL; capture {kind: design|structure|media|page|markdown|text|html|png|bundle|before|after} does what the Capture menu's item does and answers `captured` (folder, path, outcome, files count, fileNames array of receipt filenames, skipped count) with the state, whose `capture` part says what is running, the last result and the last error.

The Browser page's own menu is also available through `POST /ui/browser`:

| Action | Body fields | Result to verify |
| --- | --- | --- |
| `pointAt` | `sessionId`, `tabId`, `target: {selector}` or `{x,y}`, optional `target.quote` | A context chip; read `ui/composer` `read`, including its `label`, `source`, `text`, `by` |
| `copy` | Same target fields | `{copied}` after the clipboard write succeeds; line breaks kept, at most 100,000 characters |
| `saveToVault` | `sessionId`, `tabId` | `403 vaultUserOnly`, before any page value is read |

`tabId` must be a web tab owned by this session. `quote` selects an exact substring of that element's text;
missing elements or text are refused. Omitting it points at or copies the element's text. The core uses the
same native engine target description as the trusted menu. No route manufactures a trusted page event or
installs a page-callable owner function. The lower-level commands are `preview` `pick` and `copyText`.

### `POST /ui/changes`

Read the Changes tab of the shown session and move around in it like a user. Body: {action: state|reload|select|layout|comment|removeComment|sendComments, path?, layout?: split|unified, line?, endLine?, side?: new|old, text?, send?, id?, ids?}: state = mode (none|git|effects|unavailable), whether the tab is offered, the files with counts, notes and turn labels, the shown file's hunks (ids), turns (folders without git), the open confirmation, the diff comments; select shows a file; comment {path, line, endLine?, side?, text, send?} comments on lines of the current file (send puts a context chip into the composer, never a command); removeComment {id}; sendComments {ids?} puts the unsent comments into the composer as chips; layout switches split/unified; reload reads again. Keep and revert are not offered here: use POST /commands with {type: changes} (origin debug-api).

### `POST /ui/click`

Clicks an element like the mouse does (every control is reachable, also those without a route of their own). Body: {selector: CSS selector, index?: n-th match}. A hidden or disabled element is refused (denied). Answers right after the click; an action that waits (a native folder picker) keeps running. The click is the window's: a native confirmation it leads to opens as the window's (`GET /asks` lists it, `POST /asks/{id}` answers it; a vault confirmation waits for a person). Routes of their own answer their confirmations as yours.

### `POST /ui/controls`

Reads visible main-window controls, with optional `{selector}` (default `[data-testid]`). Add `includeHidden: true`
to inspect empty announcement regions or other mounted elements with no visible box; each result includes `visible`.
Returns `controls`,
each with `testId`, `itemId`, `tag`, `role`, `label`, `text`, `value`, `checked`, `pressed`, `expanded`, `selected`
and `disabled`. Also returns `ariaChecked`, `labelledBy`, `describedBy`, `focused`, `live` and `atomic`
for switch state, prompt labels/focus and announcement semantics. Use this after a click or a typed feature action to verify the actual field/switch/button state.
A custom selector can also read a visible section or full-request view. Secret fields return `secret: true`,
`value: null` and a withheld text marker, including when a containing section is read. Selectors run with secret
field values emptied and then restored, so they cannot query those values. Native pages and CodeMirror shadow
controls use their typed routes. `ui/key` dispatches shortcuts; input values are changed by their feature's typed
actions. `ui/list` controls the session-list visibility.

### `POST /ui/composer`

`paste {text, start?, end?}` inserts plain text at a draft range (zero-based UTF-16 offsets; start defaults
to the end of the draft and end defaults to start). It uses the window's paste path: line endings become LF, and over 20 lines or 2,000
characters fold into a chip. Invalid or reversed ranges answer `invalidParams` without changing the draft.
`read` includes `pastes` in draft order (`id`, `n`, full `text`, `lines`); it excludes removed or expanded chips.
Use `ui/click` on `composer.paste-chip-open`, `composer.paste-chip-remove`, `composer.paste-preview-expand`
or `composer.paste-preview-close` to exercise the actual chip controls. Read `ui/controls` for the preview text
and `ui/composer` for the resulting draft. Pasted content retains the Debug API origin through later window edits.

Read or act on a session's composer like typing and clicking do. Body: {action: read|type|paste|send|insertContext|attachImage|stop|retry|slash|access|model|queueEdit|queueRemove|rewind|rewindClose|branch, sessionId?: string (default: the selected session, else the new-chat view's composer "new-chat")}: type {text}; send {mode?: send|steer|queue} sends the current content (a typed !cmd runs a shell command, a GUI /command runs it; on "new-chat" it first starts the session in the folder chip's folder, applies the held access and model, then sends the first turn, and outcome.started names the new session; without a folder it refuses and keeps the text); insertContext {label, source, text, ref?} adds a context chip (always data, never a command); attachImage {name, data: base64}; retry {instructions?}; slash {name, args?}; access {mode} and model {alias?, effort?} answer with the composer as it then reads (on the new-chat composer they are held until its first send); queueEdit {queueItemId, text}; queueRemove {queueItemId}; rewind {entryId, confirm?: here|new}; branch {entryId}. The read answer's `heldByPid` is the pid of another Vibe process (for example the Vibe CLI) holding the session, while the composer is read-only, else null.

An optional `ref` names the file's absolute path and first and last line (`path`, `start`, `end`); use 0 for both lines to point at the whole file. Clicking the chip before or after sending opens that code in the File tab. This is display information; the quoted text stays data, never a command.

While the @ menu is open, `ui/controls` lists its rows as `composer.file-mention-option` and `composer.session-mention-option` (`data-item-id`: the path or the session id). A picked session is a context chip with source `session` holding that session's `sessionTools` `excerpt`.

### `POST /ui/connectors`

Read or act on the Connectors page like clicks do. Body: {action: read|open|close|filter|search|select|refresh|signIn|connect|stopWaiting|turnOff|turnOn|addServer|signInServer|signOutServer|removeServer|serverOff|serverOn|addKeyServer|allowServerKey, alias?, name?, url?, secretId?, newSecret?, header?, filter?: all|connected|signin|off, text?}: read (default) returns whether the page is open, the filter, the groups with each connector's status, the servers, the sign-in in progress (connector or server, the page address, waiting) and the selection; signIn {alias} starts a Mistral connector sign-in through the selected (else newest) session and opens the link dialog with the page address; connect {alias} is the Connect button of a connector that needs no sign-in, through the same session; addServer {url, name?} adds a server that signs in with OAuth (refused addresses are refused before any dialog); addKeyServer {url, name?, secretId | newSecret {name, value}, header?} adds a server that needs an API key and allowServerKey {name} allows its key again: these two always show the native dialog in the window, also for the Debug API, because they give a server a secret; the user's answer decides (always, the recommended choice and the dialog's default button; allow until Vibe Work closes; or not). A `newSecret` with a value, and `removeServer` of such a server, are the user's alone: refused here (`403 vaultUserOnly`, "The vault is yours"). Other changes run without the native confirmation (the Debug API acts as the user); sign-ins finish only in a browser. Server entries carry `key` (the secret's name, the header and the grant), never a value.

### `POST /ui/marketplace`

The Tools page's Marketplace tab (plugins, skill packs and MCP servers from the reviewed, version-pinned catalog). Body:
{action: read|open|connectors|reload|filter|search|select|install|offer|installOffer|cancelOffer|cancel|remove|addServer|turnOnPlugin|turnOffPlugin|removePlugin, id?, path?, filter?:
all|installed|updates, text?}: read (default) returns the tab, filter, selection, note, the install in progress, the
platform and every shown entry with its status words, installed version, update flag, access class (servers) and its
state in the viewed session (loaded, skills, Vibe's issues); open / connectors switch the tab; install {id} installs or
updates a pack or plugin without an install offer (in the background; progress arrives as `marketInstall` events);
cancel stops it; remove {id} removes it; addServer {id} adds a catalog MCP server through the Connectors flow (its
sign-in finishes in a browser).

One-click install, as the window does it: `offer {id}` is the card's **Install…**: it selects the card and shows the
install offer in the detail, right above its Install button (name, version, publisher, from, licence, what it runs).
`installOffer {id}` is the detail's **Install**: it installs exactly the shown offer (the `install` command with its
`offerId`); when no offer is shown it shows the offer first, like the click. A used or changed offer is refused by
the core (`staleOffer`), an expired one with its own code (`offerExpired`), and nothing is installed: the detail shows
the fresh offer like any offer with one sentence, "This offer changed and was refreshed. Check the details above,
then install again." or (`offerExpired`) "This offer expired and was refreshed. Check the details above, then install
again.", and never installs it by itself. `cancelOffer {id}` closes the offer (Escape).
`read` lists the open offers under `offers` (target, phase `reading|shown|installing`, `offerId`, `expiresAt`, the
heading, `facts` with `label` and `value`, the confirm words, the notice and `noticeKind` `changed|ranOut`) with read
failures and the focus request.
`discovered` lists plugins outside the catalog with their path, version and author (from the manifest; `null` when it
gives none), ownership, off state, components and current-session state. A chat reopened after one of its plugins was
removed or turned off is refreshed through `refreshPlugins`, so it no longer runs Vibe's kept copy of that plugin.
The core holds the chat through re-pin and reopen to preserve host guards; raw `plugin/reload` is refused.
`turnOnPlugin`, `turnOffPlugin` and `removePlugin` take the exact `path` returned by that inventory. The core changes
only owned plugin folders. Turn on records the folder and manifest name as approved; Turn off moves it to
`plugins-off/` and revokes approval; Remove revokes approval and deletes the folder. The window asks before Turn on
or Remove. The Debug API acts as the user without a native dialog.

Approvals live in `plugin-approvals.json` in Vibe Work's data folder, outside `vibe-home`. Marketplace Install
records approval. Before every server start and session start, resume/reopen or plugin reload, unapproved user
plugin folders move to `plugins-off/`. Existing installations without an approval also start off. Approved plugins
start with Vibe Work; project plugins retain Vibe's folder-trust flow.

A helper process that several chats share (a broker, a daemon) must not be a plain child of one chat's MCP server,
which stops when that chat closes (on Windows the MCP SDK ends its whole process job): start it as its own detached
process and let each chat's server connect to it.

Example: `{"action":"install","id":"context7"}`.

### `POST /ui/skills`

The Tools page's Skills tab, every action its controls have, through the same functions a click uses. Body:
`{action: state|open|reload|search|disclose|install|update|confirm|cancel|enable, name?, text?, enabled?}`:

| Action | Body | Drives |
| --- | --- | --- |
| `state` (default) | — | whether the tab shows, loading, the list error, note, search, busy, each row as drawn (`name`, `group` installed or catalog, `version`, `latestVersion`, `updateAvailable`, `skillId`, `enabled`, `locked`, `expanded`, `error`, `modifiedAt`: when the skill file last changed, ms since the epoch, or null) and `offers`: each open install offer (`target`, `mode` install or update, `phase` reading, shown or installing, `offerId`, `expiresAt`, `heading`, `facts` with `label` and `value`, `confirm`, `notice`, `noticeKind` `changed|ranOut`), read failures and the focus request |
| `open`, `reload`, `search` | —, —, `{text}` | the Skills tab, a fresh read, the search field |
| `disclose` | `{name}` | the row's name (opens or closes its detail) |
| `install` | `{name}` | a catalog row's **Install…**: opens the row's detail with the offer |
| `update` | `{name}` | an installed row's **Update to vN…**: the update offer (Version `4 → 5`) |
| `confirm` | `{name}` | the offer's **Install vN** / **Update to vN**: `import` or `setLatest` with the shown offer's `offerId` |
| `cancel` | `{name}` | the offer's **Cancel** (and Escape) |
| `enable` | `{name, enabled}` | the row's switch |

A used or changed offer is refused by the core (`staleOffer`), an expired one with its own code (`offerExpired`),
and nothing changes: the row shows the fresh offer like any offer with one sentence above its buttons, "This offer
changed and was refreshed. Check the details above, then install again." or (`offerExpired`) "This offer expired and
was refreshed. Check the details above, then install again." (an update offer ends "then update again."), and never
sends it by itself. A read that fails
shows "Could not read what this installs: {reason}. Nothing was installed. Try again." No native dialog (the Debug
API acts as the user). Remove keeps its dialog in the window (reach it with `ui/click`).

### `POST /ui/models`

The card that asks for a new default model when Vibe no longer offers the Settings one. Body: {action: read|check|pick|later, alias?}: read (default) returns {open, gone {alias, label} or null, models [{alias, label}] (the models Vibe offers), saving, error}; check runs the launch check again (checkDefaultModel); pick {alias} saves that model as the default for new chats like its button and closes the card; later closes it without a change (it asks again at the next launch).

### `POST /ui/documents`

Read or act on the document chips like clicks do. Body: {action: read|extract|remove|preview|asPicture, sessionId?: string (default: the selected session), docId?: string}: read returns each session's document chips (id, name, kind, status, pages, chars, kept, error) and the open preview; extract {docId} reads it with Mistral OCR (spends OCR quota); remove {docId}; preview {docId, open?: bool}; asPicture {docId} puts a PNG/JPEG/WebP/GIF into the message as a picture. To add a file, use ui/files drop (the same routing as a drop on the composer).

### `POST /ui/editors`

CodeMirror's controls inside its shadow root use these actions, with optional `sessionId` and `file`
(the focused open file by default). The file must have a mounted editor. Either side of a saved comparison
is addressed by its id in `state.files`.

| Action | Fields | Readback |
| --- | --- | --- |
| `find` | `query`, optional `replace`, `caseSensitive`, `regexp`, `wholeWord` | `code.search`: query/options, valid, matches, open |
| `findNext`, `findPrevious`, `selectMatches` | — | `code.selection` offsets, all `code.selections`, and `code.cursorLine` |
| `replaceNext`, `replaceAll` | —; uses the current search query | `code.search.matches`; `ui/files` reads the changed draft; read-only editors never change |
| `closeFind` | — | `code.search.open: false` |
| `foldCode` | `line` (1-based), `folded` (boolean) | `code.folds`: actual folded ranges with line/from/to; an unfoldable line leaves it unchanged |
| `goToLine` | `line` (1-based) | `code.cursorLine` (clamped to the document) |

`state.code` reads the focused editor's live CodeMirror state, or `null` if it is not mounted. Actions return
the target editor's `code` as well as the workspace state. `foldCode` folds code inside a document; the existing
`fold {side}` folds a whole editor pane. All these actions call CodeMirror commands/transactions directly.

The editor workspace beside the chat: the file tabs and their **+N** open-files list, two editors side by side,
**Compare with saved version**, the unsaved-changes dialog, and folding or swapping chat and sessions. Every
action its controls and keys have, through the same functions a click uses. The shown file's text and its editor
(type, save, reload) stay on `ui/files`. Body: `{action, sessionId?, ...}`; `sessionId` defaults to the session
the File tab shows (the selected session, else `files-workspace:<folder>` for a chosen folder before a chat).

- `state` (default): `files` (every open document in tab order, as the tabs and the +N list show it: `file`,
  `name`, `display`, `saved` (`saved`/`disk` for a read-only comparison side, else null), `preview` (the
  preview beside a file's code), `unsaved`, `changed`
  (Vibe wrote it), `mark`, `visible` (a tab, not behind +N), `active`), `hidden` (the +N count), `split`
  (`{left, right, focus, compare}` or null), `ratio`, `folded`, `problem` (why a comparison is not offered, for
  example a file that is not in the last commit), `closeDialog` (`{file, title, problem}` while the
  unsaved-changes dialog is open), `layout` (`editorSplit`, `slot`: chat|list|folded, `chatFolded`,
  `listVisible`, `slotWidth`), `openFilesIn` and `notices` (each open file's notice after its file changed on
  disk: `file`, `kind`: reloaded|merged|overlap, `text` as the window shows it, `lines`, `marks`, `choices`).
- `activate {file}`: a tab click, or a row of the +N list.
- `openToSide {file?}`: the split button, Ctrl \ or Ctrl Enter (the focused file when `file` is left out).
- `restore`, `focus {side: left|right}`, `ratio {ratio}` (the divider: 0.25 to 0.75), `fold {side, folded?}`
  (an editor's fold button; without `folded` it toggles).
- `compareSaved`: **Compare with saved version**: the file as in the last commit, read-only on the left, the
  editable file on the right. A plain folder, a new file or a saved version that is too large or not text gives
  `problem` instead. `closeSavedVersion`: **Close saved version** (or **Close comparison**).
- `close {file?}`: the tab's close button or Ctrl W. A saved file closes (`closed: true`); an unsaved one opens
  the dialog (`closed: false`, `closeDialog`).
- `closeDialog {choice: cancel|discard|save}`: the dialog's **Cancel**, **Discard** and **Save and close**.
  Discard drops only that file's edits. Save writes like the editor's Save: when the file changed on disk the
  file stays open with its edits, the dialog stays and `outcome` is `{kind: "kept", message}`; a newer file on
  disk is never overwritten.
- `closeSavedTabs`: the +N list's **Close saved tabs** (unsaved files stay).
- `chat {folded}` and `sessions {folded}`: the fold buttons and the folded strips' show buttons. `swap`: Ctrl B
  (sessions and chat trade the shared slot beside two editors). `openFilesIn {mode: tabs|single}`: Settings >
  Appearance > Open files in.
- `previewToSide {file?}`: **Preview to the side** for an HTML or Markdown file: its code in the left editor, its
  preview in the right (Markdown follows typing; HTML reloads on Save). The preview is an open document of its
  own (`file` `preview:<file>`, `preview: true` in `files`).
- `diskNotice {choice, file?}`: the buttons of the notice above an editor after its file changed on disk:
  `gotIt` (Got it: the marks go), `showChanges` (moves the editor to the first changed line), `undoMerge` (the
  editor text from before the merge), `keepYours` / `takeVibes` (where both changed the same lines: that side's
  lines, every other change of both kept) and `compare` (Compare side by side). A choice the notice does not
  offer is refused `invalidParams`. Only the editor changes; Save still writes.

```json
{"action":"closeDialog","choice":"save"}
```

### `POST /ui/explorer`

The Files tab (folders, History and its one search field), every action its controls have. Acting first shows
the Files tab on its folder, as a person opening it would; `state` only reads. The folder is the selected
session's folder, else the chosen New chat folder, else the project's main folder, unless a place was chosen.

- `state` (default): `folder`, `places` (`value`, `label`, `disabled`), `session` (a session is selected),
  `view`, `summary` (`branch`, `changed`, `ahead`, `upstream`, `commits`, `latest`) or null without version
  history, `historyNote`, `files` (indexed names), `changedOnly`, `showIgnored`, `ignored` (hidden ignored
  names), `rows` (the tree as drawn, at most 500: `path`, `directory`, `depth`, `expanded`, `changed`, `more` for
  a folder's next-page row; `rowsTotal`), `search` (`query`, `searching`, `names`, `hits` with `line` and
  `text`, as the window shows them: only inside the folder "Search in this folder" chose, `commits`, `limited`),
  `history` (`commits`, `more`, `pendingOpen`, `pending`: the changed paths),
  `version` (an open commit: `files`), `workspace` (the folder actions below) and `error`.
- `place {folder}`: one of `places` (an outside-project copy without a session is refused, like its disabled
  row). `view {view: files|history}`. `expand {path}` / `collapse {path}` a folder row. `more {path?}` the next
  page of a folder (the root by default). `changedOnly {on}`, `showIgnored {on}`, `refresh`.
- `search {query}`: types into the search field and answers with file names, text in files and commits;
  an empty query clears it. `openFirst`: Enter in the search field. `open {path, line?}`: a tree row, a result, a
  changed file or a commit's file (relative to the folder) opens in the File tab. Browsing and opening never
  start a session.
- `historyMore`: **Show 50 more**. `version {id}`: a commit row (or a commit result). `back`: **← History**.
  `pending {open}`: the **Changes since the last commit** disclosure.
- Before a session (in a session use `reviewSession`, which opens the Changes tab, and `ui/changes` / `ui/git`):
  `review` and `closeReview` (**Review all changes**); `commit` (**Commit…**: the Commit dialog for
  `workspace.commitKey`; finish it with `ui/git` and that key as `sessionId`); `askRevert` (**Revert all
  changes…**: the confirmation listing every file, new files marked for deletion), then `confirmRevert` or
  `cancelRevert`. The core reverts only the reviewed state: a folder that changed since is refused. A folder
  without version history: `enableHistory` (the plan: files, generated folders, large files left out), then
  `identity {name, email}` if needed and `confirmHistory`, or `cancelHistory`. These keep the window's own
  confirmation step; the Debug API acts as the user (no native dialog).

```json
{"action":"search","query":"multiply"}
```

### `POST /ui/files`

Linked names also support `ref-menu {text, item?: open|edit|reveal|copy-path, line?, file?}`. With no item it
returns the available actions, or `{choices: [{file, display, kind, size}], total}` for an ambiguous name.
Choose a returned `file` handle and an item to act on that choice; no ambiguous file opens automatically.
The chooser lists at most 50 files, with the full count in `total`. Relative names that miss are matched by
path segments inside the session folder only. `refs` marks ambiguous names as `choices` and includes their
choices. `edit` opens the File tab and starts its editor after reading complete editable text; images and binary
files have no Edit action. If the read is too large or truncated, the preview remains with an `editNotice`
explaining why the editor did not open. `reveal` uses Show in folder; `copy-path` copies the same display path
shown in the File tab and shows the usual copy feedback. These are the same actions as the linked name's
right-click, ContextMenu key or Shift+F10 menu. Each chooser row also offers Edit (or Shift+Enter).

The File tab (files the agent mentions, previewed) of a session, every action a click has (the Debug API acts as the user, no native dialogs). Body: {action: state|refs|open|switch|close|reload|mode|external|edit|drop, sessionId?: string (default: the selected session), ...}: state = the shown file (name, path relative to the chat's own folder, or the full path for a file elsewhere), kind, size, status, mode, truncation, the text it shows (up to 65,536 characters), the viewer and the recent list, and the last drop report; refs {entryId?} = the file references the transcript shows (text, path, line, state ok|no|pending); open {text, line?} resolves a path as the transcript does (the window's scope: the session folder and picked folders), or open {file} a handle from refs or the recent list; switch {file}; close; reload; mode {mode: rendered|source}; external {what: open|reveal} opens with the default app or shows in the folder (a program is never opened); drop {files: [{name, data (base64), folder?}]} routes files like a drop on the composer and answers the drop report and the composer's pictures, context chips and document chips. Known secrets in a file's text are hidden like in every answer (see the top of this page). edit {text} saves the shown text file with that whole text like the editor's Save: refused with `stale` when the file changed on disk since the tab read it; line endings and a byte-order mark are kept; the agent gets a context chip with the diff.

The code editor also supports `openEdit` (open the shown complete text file without saving), `setText {text}`
(replace the editor buffer by a transaction, including an empty string), `saveEdit`, `cancelEdit`, `reloadEdit`
(explicitly discard the buffer and load disk), `keepEditing` (dismiss the conflict banner; retain the original
save fingerprint), and `compare` (open a read-only Mine/Disk comparison). These use the same draft and save actions
as the window. `saveEdit` returns `outcome: saved|unchanged|stale`; a stale save keeps the draft and exposes the
conflict in its state. It never overwrites the newer disk version.

With an unsaved draft, `open` and `switch` select a pending preview while the editor keeps its original file,
text and save fingerprint. The window explains which file is still being edited. Save or Cancel exposes the
pending preview; `close` is refused until the draft is saved or cancelled. Session switches retain each session's
draft. Comparison and `reloadEdit` read the edited file even when the pending preview names another file.

`state` and action replies include `editor: null` when closed, or `{text, textCut, dirty, conflict, problem,
saving, compare, wrap, baselineFingerprint, diskFingerprint}`. Buffer text is limited to 65,536 characters in
replies. A clean buffer reloads in place when disk changes; a dirty buffer keeps its baseline and text. Saved user
edits are protected from the session Changes view's agent revert. Ambiguous overlapping changes conservatively
accept the saved file into that review baseline.

Editor buffer text uses LF line endings, including `setText` input. Saving preserves the disk file's existing
line-ending style and byte-order mark through the core's file-write path.

### `POST /ui/git`

The branch bar at the top of the Changes tab, the Commit / Pull request / Bring into main dialogs and the PR card of the shown session, every action a click has. Body: {action: state|reload|selectRepo|openCommit|setMessage|toggleFile|runHooks|commit|push|openPr|setPr|createPr|refreshCard|sendChecks|sendReview|openBring|bringIn|askVibe|close, sessionId?, repo?, text?, path?, on?, then?: none|push|pr, remote?, title?, body?, base?, draft?}. `selectRepo` picks a root returned by status (for example `sub`). `state` (default) returns the selected `repo`, discovered `repos`, whether the bar shows, the status (branch, base, upstream, remotes, files to commit, why Commit is blocked), the PR card, the open dialog (commit: the message and where it came from, the files with `included`, the author, the repository's commit hooks and `runHooks`, `credit`; pr: the plan, title, body, base, draft; bring: the dry run), busy, the last notice and what the last commit printed. Every action uses the selected repository. `openCommit` drafts a message (Mistral with your key, else the session title); commit {then?} commits the included files and can continue with a push or the PR dialog; push, createPr and bringIn carry the Debug API origin, so the core's confirmation is answered as yours and only what it names runs; sendChecks, sendReview and askVibe put a prepared message into the composer as a chip, never sent.

### `POST /ui/git-browser`

The Git side panel browses the selected session's saved project, or the project chosen in the panel on the
new-chat view. Body: `{action: "read"|"click", projectId?, nodeId?, target?, path?}`. `read` returns a fresh
`snapshot` with the project folder, repository and copy nodes, branch, changed files, upstream counts, plain state
words, restrictions and the existing session/repository action selectors. Discovery is bounded; `truncated` and
`note` explain incomplete discovery or file watching. Outside-project copies are shown as locations only.

`click` requires a `nodeId` from that project's current snapshot. `target` is `primary`, `commit`, `push`, `pr`,
`bring`, `new-chat`, `new-task` or `file`. `primary` chooses the node's current next step. `file` also requires a
changed `path` from that node and opens the existing File tab or Changes diff. Git actions reuse the existing
dialogs and core guards; a push's confirmation is answered as yours, the same as `ui/git`.
An unavailable action returns its reason. Nodes without an admitted session explain that a chat must be started
there before Git actions are available. New chat prepares that folder with the copy switch off; New task prepares
the same composer with the copy switch on. The node's folder counts as chosen, like `ui/worktrees` `folder`. Neither sends a message or creates a copy until the user submits.

```json
{"action":"click","nodeId":"<id returned by read>","target":"new-task"}
```

### `POST /ui/key`

Presses a key like the keyboard does. Body: {key: e.g. Escape|Enter|k, code?: physical key such as KeyK, selector?: CSS selector to focus first, ctrl?, shift?, alt?, meta?}. Without `code`, letters, digits and punctuation use the shortcut registry's physical key names. Without a selector the key goes to the focused element. Like `ui/click`, a non-vault confirmation it leads to can be answered through `POST /asks/{id}`; vault dialogs require the person in the window.

### `POST /ui/menu`

The app's action menus: one grammar everywhere (a file name in the chat, Files rows, History, search hits, file
tabs, the editor, Changes rows and blocks, the session list, the terminal, a selection in an answer). A right
click, the menu key, Shift F10 and a row's ⋯ open the same menu. Body: {action: state|open|choose|close,
selector?, index?, item?}: state (default) answers the open menu or `{open: false}`: its testId, label, heading
(the line naming the target: a path, `path:line`, a commit) and items (id, label, keys as shown, disabled,
danger) in the order shown; open {selector, index?} right-clicks that element like the mouse does (a hidden
element is refused) and answers the menu it opened; choose {item} runs one item exactly as a click does (a
disabled item is refused) and closes the menu; close is Escape. The menu key is `ui/key` with
`{key: "ContextMenu"}` or `{key: "F10", shift: true}`; a row's ⋯ is `ui/click`. The Project tab's page is a
separate native view: its menu (Point Vibe at this, Comment…, Copy for selected text) opens on a person's right
click in the page, which reports only the point and whether text was selected (event `pageMenu`); once open,
`state`, `choose` and `close` work as for every menu. Its items run the `pages` requests `menuSection` and
`menuCopy`, which read the named section/quote when supplied, or the last trusted right-click target otherwise (nothing without a target). The Browser tab's pages have their own menu inside
the page (Point Vibe at this, Copy, Save to vault), out of this route's reach.

```json
{"action":"open","selector":"[data-testid='explorer.tree-row'][data-item-id='src/calc.rs']"}
{"action":"choose","item":"point"}
```

### `POST /ui/layout`

Automatic layout checks of what is shown (the visual pass). No body. Returns the viewport, every shown menu, listbox, dialog, tooltip and popover with whether it is fully inside the window, buttons and headings whose text is cut off, horizontal scroll, and a problems list.

### Slow UI actions and `POST /ui/requests`

UI actions that take more than 50 ms acknowledge with
`{pendingId: "ui-…", state: "pending", poll: "/ui/requests"}`. Poll `POST /ui/requests` with
`{pendingId}` to get `pending`, `completed` with the original `value`, or `failed` with the original typed
`error`. Polling does not enter the UI queue. A request id is a correlation id, never an authorization capability.
Pending work (with what the request may do as yours) and completed results are retained for at most ten minutes; the host keeps at most 128
results. An unknown or expired id answers `404 notFound`.

`/ui/state` and `/ui/libraries` `read` remain available during work. Another UI action answers
`{pendingId, state: "busy", poll: "/ui/requests"}` without starting it; collect the earlier result before retrying
that action. This covers awaited handlers and commands started by clicks, including provider calls, downloads,
file operations and native dialogs. Fast actions keep their usual result. Libraries `ask` can finish normally
after the UI request's ten-second limit; its answer is available in the completed value and Libraries `read`.

A slow action keeps that one busy slot for at most ten minutes, the same time the host keeps its result. If nobody
ends the wait by then (for example a vault dialog that you do not answer), the window lets go: the next
action runs again, the window's commands are no longer recorded as the Debug API's, the late answer is dropped
and the id answers `404 notFound`.

### `POST /ui/libraries`

Read or act on the Libraries page like clicks do. Body: {action: read|open|close|select|create|scope|delete|upload|deleteDocument|text|closeText|ask, libraryId?, documentId?, name?, scope?: off|global|project:<id>, files?: [{name, data (base64)}], question?}: read (default) returns whether the page is open, the libraries (id, name, documents, scope: who may use it in chats), the sentences about what is sent and what Mistral charges, the selected library's documents (id, name, Mistral's processing status, pages), the note, the last answer with its numbered sources and the open document text (its start); create {name, scope?} makes a library on Mistral (default scope global); upload sends the files' bytes to Mistral (like a drop; there is no path action, the window's Add files uses the native picker); delete deletes the library and its documents on Mistral; ask {question} asks across every library that is not off (spends Mistral calls). No native dialog (the Debug API acts as the user).

### `POST /ui/list`

Show, hide or toggle the session list. Body: {action: show|hide|toggle}. Returns whether it is visible.

### `POST /ui/notifications`

Read or set the notification state. Body: {action: read|level|focus}: level {level: errors|permissions|all}, focus {focused: bool|null} overrides the window-focus signal for tests (null = real). Returns the level, the focus override and the OS notifications the UI asked for since the page loaded. Each request contains a short body summary and may include a `notificationId` linking activation to the complete message retained in the app's Notifications panel.

### `POST /ui/pages`

Alias of ui/project for one release (the Project tab was called Pages in earlier versions). Same body and answer.

### `POST /ui/palette`

The command palette, like the keyboard uses it. Body: {action: list|open|close|run, query?, id?}: list (default) returns the commands (id, label, section) the query matches; open/close show or hide it; run {id} runs one exactly as Enter does (like `ui/click`: a non-vault confirmation can be answered through `POST /asks/{id}`; vault dialogs require the person in the window).

### `POST /ui/panel`

Open, close or toggle the side panel and pick its tab. Body: {action: open|close|toggle, tab?: project|plan|browser|git|changes|terminal|file} (git is the Files tab; pages = project, an alias for one release). Returns the panel state: `open`, `tabs`, `active`, `width` and `maximized`.

### `POST /ui/plan`

The Plan tab (the plan checklist beside the chat) of the shown session. Body: {action: state|open, step?}: state (default) returns the current plan page (path, title), its steps (id, status, text), progress (done, total, doing) and whether the tab is open; open {step} does what a click on a step does (the Project tab at that step of the plan page).

### `POST /ui/processes`

The "Left running" list: processes the agent started that keep running after their session was deleted. Body: {action: read|open|close|stop, id?}: read (default) returns {open, items: [{id, sessionId, sessionTitle, pid, name, ports}], stopping}; open/close show or hide the list dialog like the status bar button and Close do; stop {id} stops one listed process like its Stop button (no dialog: the request is the click).

### `POST /ui/project`

The handbook page's right-click items can be driven without opening its native menu:
`pointAt {sessionId, section, quote?}` reads the named section of the shown page and adds the menu's context
chip; `copy {sessionId, section, quote?}` writes the selected text to the clipboard and returns `{copied}`.
The section must exist in the actual page file, and an optional exact quote must occur in its rendered element.
Read the chip's full text and origin with `ui/composer` `read`. `comment {path, section, text, quote?}` stores
the same section comment the menu's form submits; read `ui/project` state to verify it. The core commands
`pages menuSection` and `menuCopy` accept `section` and `quote`; without them they retain their existing read
of the last trusted right click. A webpage cannot create that click by dispatching an event.

The Project tab (the project handbook) of the shown session. Body: {action: state|hub|open|plan|back|forward|reload|pick|comment|send|delete|point|pointAt|copy|answers|answer|sendAnswers|showAnswers|check|export, ...}: open {path, anchor?} (anchor: a section or step id to scroll to); comment {path, section, text, quote?, send?}; send {id}; delete {id}; point {source: terminal|preview|page} ("Point Vibe at this": a context chip in the composer, never a command); answers {path?} (the page's inputs with the user's answers; every page with inputs without path and no page shown); answer {path?, input, value} (value: {type: choice, option} | {type: checklist, checked: [...]} | {type: rank, order: [...]} | {type: text, text} | null to clear; checked against the page file); sendAnswers {path?} (every answer of the page into the composer as one chip, never a command); showAnswers {shown} (true: the Answers panel shows every control; false: only its one line with the counts, Send to Vibe and Show answers; one choice for the window, the state's answersShown); check {path?} (the visual check now: desktop 1280 and phone 390 px, light and dark); export {path?, destination?} (the page as one self-contained HTML file: its text, or written to the absolute destination like the save dialog). path defaults to the shown page. `pick {section}` picks the named section of the shown page at once and answers `{picked, ...state}`; `point` names its element with `target` (`{selector}` or `{x, y}`, source `preview`) or its `section` (source `page`). Without them the answer is `400 invalidParams` naming the field: you have no pointer, so you name what a person would have clicked. Its calls are the window's, recorded as the Debug API's; an export with destination is written without the save dialog. Returns the tab state (hub with manifest, findings counts and groups, or page with steps, sections, comments, answers, visual check, prototype; the viewer) and the action's result.

### `POST /ui/projects`

The Projects overview: every project's handbook manifest, active plans and health without opening a session. Body: {action: state|open|close|refresh|select|filter|query|openHandbook|start|newSession|openFiles|openTerminal|reveal|chooseFolder|newProject, ...}: select {projectId|null}; filter {status: all|active|maintenance|paused|archived}; query {text}; openHandbook {projectId, path?} (a session's Project tab); start {projectId} reuses the project's current chat (or starts one) and sends the handbook prompt as a user message, leaving any composer draft untouched. While the chat is working, the handbook request is unavailable; openFiles {projectId} opens the New chat screen in the project’s main folder with Files selected; openTerminal {projectId} opens the New chat screen in the project's main folder with its Terminal tab (no session starts); reveal {projectId} shows the project folder; chooseFolder {projectId, folder} sets a folder (a path, or null to clear; `folder` is required: the native picker cannot be answered remotely); newProject {name} creates a project. `newSession` takes `folder` (a path) for a project without a main folder, else it answers `invalidParams`: give the folder, or set one with `chooseFolder` first. Returns the page state (cards with name, state, status, plans, health).

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

Act on the session list like a click does. Body: {action: read|filter|archived|more|refresh|open|rename|pin|archive|delete|retryReopen|cancelReopen|continueNew|temporaryChat|toggleGroup|projectCreate|projectRename|projectDelete|projectMove|projectPlace|projectStart|projectFolder|projectsRefresh, ...}: filter {text}, archived {show: bool}, more (load the next page), refresh, open/pin/archive/delete/rename {sessionId, ...} (pin {pinned: bool}, archive {archived: bool}, rename {title}); retryReopen, cancelReopen and continueNew act on a stuck session; delete runs without the native confirmation (the Debug API acts as the user) and also answers leftRunning: the processes the deleted session left running (they are not stopped, use ui/processes stop). A subagent chat whose parent chat still exists is refused with "Subagent chats are removed when you delete their parent chat."; a branched chat, or a subagent chat whose parent is gone, is deleted. temporaryChat {on: bool} sets the new chat's "Temporary chat" switch. The project actions create, rename, remove, move or place groups and sessions, start a chat in a project, choose its folder and refresh groups; each uses the same project id or name fields as the matching action in the window. Returns the list model (groups, pinned, needs-you, unread flags, badges), the temporary session ids as `temporary` and the switch as `newChatTemporary`.

`projectStart {projectId}` uses the same project chat action as `ui/projects` → `newSession` above and returns
`sessionId` (null on cancellation/failure) with the updated list model. For a project without a main folder both take
`folder`, the path the chat runs in (registered like **Choose folder**); without it they answer `invalidParams`, since
the native folder picker cannot be answered through the Debug API (or set the project's folder first with
`projectFolder` or `ui/projects` → `chooseFolder`).

### `POST /ui/settings`

Read or act on the Settings page like clicks do. Body: {action: read|open|close|category|search|statusUsage|instructions|errorDetail|cliSharing|sendTo|allowed|chrome}: chrome {do: read|level|offer|setup|store|remove, level?, offerId?} sends what Settings > Browser > Your Chrome's level cards, Set up dialog, store button and Remove send (a typed level change uses the Debug API's owner rights; generic clicks still open a non-vault ask answerable through `POST /asks/{id}`); read (default) returns whether the page is open, the category list and the shown category, the search inside Settings and its results (id, category, label, path), the shown category's rows as the user sees them (id, label, value: a switch's state, a pressed segment or a selector's choice, lit), the About facts (version, author, contact mailto: target, source link and project page targets), the personal instruction editor (document facts, draft text, dirty, review step, busy, notice, error) and the error-detail preference; cliSharing {share?: bool} saves the CLI session-sharing switch when `share` is given and returns it with the CLI home and both Vibe versions (applies at the next start); sendTo {on?: bool, notNow?: true} is General > Right-click menus ("Send to Vibe Work"): `on` adds or removes the system's right-click entries exactly as the switch does (the start screen's Add is `on: true`), `notNow` is that row's Not now, neither reads; it returns `sendTo` (system, supported, on, underMoreOptions, offer: whether the one-time row shows) and `sendToError` (the sentence the page shows, or null); open {category?, setting?} shows the page at a category (with `setting`: at that setting, lit for a moment), close hides it (the view underneath shows again unchanged); category {id} shows one category; search {query} types into the search inside Settings; statusUsage {show?: bool} reads or sets the Appearance switch for the status bar's context readout; markVibeLines {on: bool} (Appearance > Mark lines Vibe changes in open files) and saveBeforeSend {on: bool} (General > Save open files before Vibe works) set the open-files switches, both returned by read; openHtmlMarkdownAs {mode: preview|code|side} (Appearance > Open HTML and Markdown as); instructions {do: load|type|review|back|discard|save|loadCli, scope: personal, text?} edits the personal AGENTS.md through the review step (`loadCli` is Add my terminal's AGENTS.md: it adds that file's text below the draft, once), which shows what Save writes (save writes the reviewed draft only; a person's Save in the window writes at once; open sessions then use it from their next message, after their current work finishes); errorDetail {mode: plain|details}; allowed {do, ...} reads or acts on Settings > Allowed without asking and the "How much Vibe may do without asking" section of Settings > Safety, returned as `allowed` (the count line, each group with its count, open state, read problem, locked vault, status and refusal lines, its rows shown and its bulk link, the open confirmation, the Safety summary line): do read (default), review (the summary line's Review), open {sessionId?} (opens the page; with a session, its answers' sub-group), level {row: access|chrome|askFirst|closeTabs} (a level row's link), find {query} (also matches a project's folder name; rows are in one order, last used first), clear (Clear the search), toggle {key} (a group or sub-group header: sessions, sessions/{session id}, earlier, vault, vault/commands, vault/fills, vault/servers), more {key} (Show 20 more), revoke {key} (a row's Revoke; keys as `read` lists them), revokeAll {key} (opens the confirmation), cancel, confirm (one bulk call), openSession {sessionId}, openSecrets. Vault revokes are the user's own: under UI control the core refuses them `403 vaultUserOnly`, the route answers that refusal and the page shows the same words; nothing is revoked.

### `POST /ui/settle`

Waits until the UI is quiet. Body: {quietMs?: 250, timeoutMs?: 5000 (max 30000)}: no DOM mutation for quietMs, no running animation, two frames painted. Returns {revision, settled, waitedMs}; settled false means the timeout came first.

### `POST /ui/setup`

Read the Mistral key screens. No body. READ-ONLY: returns whether a key service exists, whether the setup screen covers the content, the step (create, pickVault, unlock, key, ready, blocked), the vault and key states, the last test outcome and failure reason word, and counts of saved keys and Vibe's own key places. Never a key or a password; changes go through the `keys` command.

`resultPending` is true while a setup test runs or its result needs reading: a failed or idle test,
a removable Vibe copy, or a credential-store `rememberNote`. A successful test with neither offer nor note
clears `resultPending` and continues automatically. The last `test` still reports `phase: "ok"` and `models`;
`covers` becomes false (Vibe installation may still show its own setup step). Its one confirmation is
observable through `POST /ui/toasts`: "Key works.", prefixed with "Vault created. " after creating a
vault. Drivers supporting older builds should wait for
either setup to finish or `setup.continue` to appear, and click Continue only when it appears.

The `keys.readStatus` command reports `availableVaults`, `vaultSelected` and `vaultPath` (null before
selection), `rememberedDevice` (the vault's key is kept in this computer's credential store, so it opens unlocked at the
next start) and `lockVaultOnQuit` (the Settings switch; changing it is the window's, `setLockVaultOnQuit` is refused
here). Vault kinds are `vibeWork` (private app-data vault), `shellx` and `desktop`.

<!-- vocabulary:vault:start -->
The window names them **Vibe Work vault**, **ShellX Vault** and **Vault app**. Its per-project entry
permissions are **Ask**, **Always** and **User only** (`askEachTime`, `allowed`, `never` on the wire).
**Pause agent access to secrets** pauses agents' use of secrets while sessions keep working;
**Lock vault** closes the vault (its contents stay encrypted) and pauses model requests until unlock.
<!-- vocabulary:vault:end -->

These fields contain only kind and folder metadata. Vault management commands remain user-only.

Vault entry listings include `defaultPermission`, explicit `projectPermissions`, and
`projectPermission` for the chosen permission project (null means inheritance).
The effective `permission` uses that project's setting or the entry default, with
User only on the current default or either project candidate winning. Older entries default to Ask.
Every request reads permission fresh, including in open chats. Secret runs use Allowed entries without a prompt;
Ask opens Deny / Allow once / Always. Always sets the requested entries to Allowed for that project.
User only entries are absent from agent lists and guessed names answer exactly like missing entries.
The agent's secret list contains no permissions or command grants.
`secrets.setDefaultPermission` and `secrets.resetEntryPermission`, like the project
permission setter, are user-only and return `403 vaultUserOnly` to the Debug API.
Replacing or generating a replacement value also requires native confirmation.

### `POST /ui/state`

Read the shell state: theme, list visibility, selected session, panel, the overlay that covers the content (settings, palette, ...), the toasts on screen, the app version (appVersion, as Settings > About shows it; null before the core answered) and the session rows with their status badges.

### `POST /ui/terminal`

The Terminal tab of the shown session. Body: {action: state|selection|selectAll|focus|open|select, terminalId?}: state = its terminal tabs, the active one, whether a terminal has keyboard focus, font size and renderer; selection = the selected text of the active terminal (read only, for "Point Vibe at this"); selectAll selects the whole buffer; focus requests focus, waits one event tick, and returns `focusReason` (`null` when focused, otherwise a missing view, hidden window, unfocused window or input that did not receive focus); open opens a new terminal tab (same as the + button); select makes terminalId the active tab.

### `POST /ui/theme`

Set the theme. Body: {theme: dark|light|toggle}. Returns the theme.

### `POST /ui/toasts`

The toasts on screen. Body: {action: read|dismiss, id?}. Returns the list (id, kind, text), including each toast's complete text even when its view shows a preview. Dismissal removes the active toast; the Notifications panel retains its message for this app run (up to 100 recent entries).

### `POST /ui/transcript`

Read or act on the transcript like a click does. Body: {action: read|open|answer|protected|chrome|grants|grantsOpen|revokeGrant|grantsAllowed|link|sources, sessionId?: string (default: the selected session)}: chrome {askId, choice: once|session|always|no, note?} presses a button of a Chrome Control card (read lists each card row with `askId`, `open`, its `question`, `buttons`, `covers`, `reason` and, once closed, its `audit` words); read returns the rows as rendered (kinds, keys, entry ids, web sources per answer and search step; an open card's `question`, `deadline`, `buttons` in their order with their keys (`once`, `session`, `always`, `no`), the `covers` words of "Allow for this session", and `choices` as Vibe's decision names; a protected change row's `proposalId` and `status`); open {key, open?: bool} opens or closes a group, step or show-all; answer {callbackId, choice?: once|session|always|no, decision?: approve|approve_for_session|deny, feedback?, answers?: [{labels: [string], other?: string}], cancel?: bool} answers a card exactly as its buttons do, and only with a button the card shows (no "Stop the turn", no "Always allow"; `always` first sets the Settings control the card names, which may open that control's own confirmation); protected {proposalId, choice: once|no} is Allow once / No on a protected change card; grants returns the session header's grants chip and list (count, open, each grant's words, detail, uses, last use, the status line); grantsOpen {open?: bool} opens or closes the list; revokeGrant {id} is a row's Revoke (the next matching call asks again); grantsAllowed is the list's "See everything allowed without asking"; link {url} shows the link dialog; sources {key, open?: bool} opens or closes the side list of an answer's web sources. Answers through this route are recorded as the Debug API's (`debugApi`), so they also show in the Security group of the bell.

### `POST /ui/updates`

Read or act on the update flow like the notice and the Settings section do. Body: {action: read|check|download|install|autoDownload|later, enabled?: bool}: read (default) returns whether this build has an update channel, the flow's view (phase: idle|checking|available|osTooOld|downloading|ready|installing|restarting|failed, version, notes, progress, mode inApp|linkOnly, autoDownload, releasePage; osTooOld = the new version needs a newer system, nothing is downloaded: os (macOS, Windows or Linux), minimum and this computer's current version; Windows versions are builds such as 10.0.19045, Linux versions are the C library's, glibc), the status line and the notice text; check asks the update server now; download and install click the same buttons as the window (no dialog either way: the request is the user's agreement); install only works for a downloaded and verified (ready) update and restarts the app; autoDownload {enabled} is the Settings switch; later hides the notice.

### `POST /ui/voice`

Read or drive dictation, narration and voice chat like the mic button, the narration bar and the Voice chat button do. Body: {action: read|start|stop|cancel|readAloud|narrateLast|stopNarration|skipNarration|settings|voiceChat|voiceChatEnd, sessionId?: string (default: the selected session)}: read returns the dictation state (state, streamId, mode, seconds, live text, last inserted text, error, micPending, permissionWait) and narration (enabled, playing, queue, last played: chars, bytes, seconds); start turns the microphone on and answers once it records, failed, or waits for the microphone permission (micPending; an OS prompt has no time limit, poll read); stop inserts the text at the caret of the session's text box (marked as text from an external client); readAloud {text?} reads the last finished reply or the requested text once, without changing voice settings, and returns before audio finishes; narrateLast narrates the session's last finished turn; settings {narration?, voice?, language?, micId?, voiceChatAutoEnd?, voiceChatPauseMs?} changes the Settings screen's voice choices. Voice chat: voiceChat {on: true|false} turns it on for the session (moving it from another session) or off; voiceChatEnd ends the user's turn now, like pressing the button while it listens; read also returns voiceChat (on, owner, phase: off|listening|sending|thinking|waiting|speaking|error, micPending, live text, what was heard, endedBy: button|pause|limit, the last spoken text with its length and seconds, the spoken count, note, error, autoEnd, pauseMs).

The owner may also use `POST /commands` with `{type: "speech", request: {op: "readAloud", sessionId, text: null}}`.
This queues one reading in the open window using the Narration voice. It returns `started`, `nobodyListening`,
`keyMissing`, `voiceUnavailable` or `noReply` with one plain sentence. The `readAloud` event carries the session and requested reading
to the window; its internal `readingReady` receipt accepts the first speech call before audio is queued. Session agents use the `read_aloud` tool with their own session credential; they cannot select another
session. Skip skips the current reading; Stop silences it and clears the queue. No voice setting changes.

The owner can request another reading in the same user turn through either Debug API route: the owner asks explicitly,
so the agent's once-per-message limit does not apply. The agent can start one reading per user message in its own
session, including when it supplies text. Another real user message allows one more; automatic harness messages do not.
A repeat returns: "This reply was already read aloud; ask again in a new message to hear it again."
At most 2 requested readings can be outstanding across all sessions in the window, including speech being prepared:
one playing and one queued. This window limit applies to the owner too. A further request returns `voiceUnavailable`
with "Another reading is still playing." before making another speech call. Skip, Stop or a finished reading frees space.

### `POST /ui/worktrees`

Read the worktree UI. Body: {action: state|folder, path?}: state = the new-chat view's folder (newFolder, newFolderBy), the open removal inspection (session, the inspection: files a removal discards, branch plan, why it is blocked) and `orphans`, the same inventory as the core `worktrees` op `orphans`: leftover copies (deleted session, undecided) and copies retained by older versions, excluding removed/missing copies and copies used by live sessions; state refreshes the inventory; folder {path} chooses a folder for the new-chat view like Choose folder does (pickFolder: it counts as picked for this run). Start and remove go through POST /commands with {type: worktrees} (no native dialog for the Debug API). A `.worktreeinclude` file at the repository root copies the git-ignored files it names into the new copy: `options` returns what would be copied (`include`, names and sizes only) and `start` returns what was copied and what was skipped and why (`include`); file contents are never returned. `options.existingBranch` names the branch when the chosen folder is already a Vibe Work copy; the new-chat switch defaults off in that folder. Saved copies can be chosen from the welcome list with New chat here; the sessions footer Copies count opens that same list.

### `POST /ui/ui-client/poll` and `POST /ui/ui-client/reply`

The transport a UI running in a browser (the developer UI) uses instead of the window's own bridge.

## Evidence workflow

1. `POST /ui/revision` (note `revision`), act (a command or a UI route).
2. `POST /ui/settle` with `{"quietMs": 300}`: it answers when the window stopped changing.
3. `GET /screenshot` (and `POST /ui/layout` for automatic checks: menus inside the window, no cut-off text, no
   horizontal scroll).

### `POST /ui/walkthrough`

A caption for a Debug API walkthrough, above the status bar at the bottom-left. Normal use has no caption.
Body: `{action: read|start|step|pause|finish|stop, total?, step?, label?}`. `start {total}` enables it;
`step {step, label}` announces a control before the click, for example `Step 12 of 340 · Settings > General > Save`.
`read` returns `{phase, step, total, label}`; phase is `off`, `running`, `paused` or `finished`.
Esc or `pause` leaves `Walkthrough paused at step 12`; a runner must read the phase and stop its pacing.
`finish` shows `Walkthrough finished · 340 steps` for three seconds. `stop` removes the caption immediately.
Step results belong in the runner's timestamped log, never in the caption. The caption ignores pointer input and
is hidden from assistive technology.

A protected-folder review is decided only through the owner command, for example
`{"type":"decideProtectedChange","sessionId":"s1","proposalId":"<id from the review>","approve":true}`.
`approve:false` is a No: nothing is written, the proposal call returns the declined result and the turn goes on. This command has no MCP tool
equivalent. The agent has only `propose_protected_change` and `protected_change_status`. A proposal call waits up to
25 seconds for the owner decision; a review still open then answers `{"status":"waiting","proposalId":...}` and stays
open, and `protected_change_status` with that `proposalId` waits again the same way and returns the finished
outcome. Reviews and final
records are retained in the session history as `protected_change` entries; a review still pending at an app restart
is recorded as stopped (the chat shows Stopped) and cannot be applied. Stop declines all pending reviews in the stopped session.

## OS “Send to Vibe Work” hand-over tests

The file-manager entries and macOS Services are outside the app's DOM. A test invokes the installed app's own
CLI, with the same isolated profile and OS environment as the running instance:

```sh
vibe-work --attach /absolute/path/to/file.txt /absolute/path/to/folder
printf '%s' 'Selected text from another program.' | vibe-work --attach-text -
```

The second process hands the request to the existing instance through its hand-over inbox and exits. With no
instance running, it starts the app and drains the same inbox once the UI connects. This is the path used by the
OS entries; it does not send a model turn. Read `POST /ui/composer {"action":"read","sessionId":"new-chat"}`
for the resulting `context` chips (`label`, `source`, `text`, optional `ref`, `by`) and draft/note. A missing file
is reported in the note; repeated files are not added twice. `by` is `debug-api` only for Debug API-created chips;
an OS/CLI hand-over has the normal owner origin. Verify `running` is null and the session list has not grown.
Use the platform's actual file-manager/Services test separately to prove registration and native event delivery.
`ui/settings {action:"sendTo",on:true|false}` controls and reads registration, including errors.

Startup asks for a saved linked New chat folder or a saved Chrome permission level remain pending until answered;
read `GET /asks` and answer the non-vault ask with `POST /asks/{id}`. A saved file is never its own consent.
Vault actions and vault dialogs retain their person-only boundary.

Settings `chrome` actions require a build carrying the Chrome Control package. Without it, every action replies: "Chrome Control is not in this version of Vibe Work." The Browser and Safety pages and Settings search omit its entries.
