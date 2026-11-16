<!-- Built-in browser argument reference; uses browser_act and browser_read, linked by the browser skill. -->
# Browser action arguments

`browser_act {action, tab?, ...}`, arguments directly in the call. Refs come from the latest `browser_read`; stale refs are refused. "Ask" = runs without a card in Ask; "asks" = card in Ask.

## All actions

| Group | `action` names |
|---|---|
| Pages, tabs | `navigate`, `back`, `forward`, `reload`, `list_tabs`, `close_tab` |
| Input | `click`, `page.click`, `page.double_click`, `page.right_click`, `page.middle_click`, `page.triple_click`, `hover`, `drag`, `move`, `wheel`, `resolve_point`, `scroll`, `scroll_position`, `type`, `insert_text`, `focus`, `blur`, `select`, `check`, `press`, `upload`, `form`, `highlight`, `wait` |
| Dialogs, sign-in | `page.dialogs`, `page.dialog.accept`, `page.dialog.dismiss`, `page.dialog.prompt`, `auth.basic`, `fill` (skill item 6) |
| Downloads | `files.download`, `files.wait_download`, `files.list_downloads`, `files.save_download` |
| Windows, screenshots | `window.open`, `window.focus`, `window.close`, `window.list`, `window.resize`, `capture.screenshot` |
| Batch | `{steps: [...]}` |
| [advanced.md](advanced.md) | `tabs.emulate`, `recording.start`, `recording.stop`, `recording.video.start`, `recording.video.stop`, `cookies.metadata`, `cookies.read`, `cookies.change`, `storage.metadata`, `storage.read`, `storage.change`, `clipboard.read`, `clipboard.write`, `clipboard.read_rich`, `clipboard.write_rich`, `media.read`, `media.play`, `media.pause`, `media.seek`, `media.mute`, `media.transcript`, `tabs.audio`, `tabs.mute`, `console.read`, `console.clear`, `page.errors`, `page.errors.clear`, `network.list`, `network.headers`, `page.inspect`, `raw.execute`, `page.add_style` |

## Pages, tabs and input

| Action | Arguments | Rule |
|---|---|---|
| `navigate`, `back`, `forward`, `reload` | `{url}`; reload `bypassCache: true` | Reload receipt: page and whether still loading |
| `list_tabs` | more: `tab: result.nextTab` | `tabs`: id/site/owner/active/suspended; title/URL under `untrustedPageContent.tabs` |
| `close_tab {tab}`, `hover {ref}`, `drag {ref, to}`, `check {ref, checked}`, `press {key, ref?}` | | |
| `click`, `page.click`, `page.double_click`, `page.right_click`, `page.middle_click`, `page.triple_click` | `{ref}` or `{x, y, ref?}` (viewport CSS px); `modifiers: ["Alt", "Control", "Meta", "Shift"]` | These use checked native input; a failure after input says it may have acted and must not be repeated blindly |
| `type` | `{ref, text, submit?}` | Replaces text; submit presses Enter; `clear:false` inserts like `insert_text` |
| `insert_text`, `focus`, `blur` | `{ref, text}` / `{ref}` | Insert: at the selection, no key events, keeps surrounding text, reads the value back; focus/blur verify focus. Password and other secret fields remain unavailable |
| `select` | `{ref, value}` or `{ref, values}` | Multiple native options; `values: []` clears them |
| `upload` | `{ref, paths}` | Files from this chat's folders; `paths: []` clears selected files; file pickers never open for you |
| `form` | `{fields: [{action:"type",ref,text}, {action:"select",ref,value}, {action:"check",ref,checked}]}` | 1–16 fields, all checked first, each read back; a failure stops later fields. Submission is separate. The call uses the highest shared access requirement of its fields |
| `wait` | `selector` or `ref` + `state: attached\|detached\|visible\|hidden`, or `text`, exact `url`, `load: interactive\|complete`; `timeoutMs` 1–15000; `ms` 1–10000 = plain delay | Receipt says whether the condition was observed; selectors: main document; refs keep frame/node |
| `scroll`, `scroll_position` | `x`/`y` deltas, or `direction` (incl. left/right) + `amount`; `{ref}` alone scrolls it into view | Ref = that pane, else document; `scroll_position` reads offsets only. Receipt: offsets, extents, edges, movement |
| `move`, `resolve_point` | `{x,y}` of the latest observation or `{ref}` | `move`: native pointer, no button; `resolve_point`: target under a point (ref, bounds), no input. Node, frame, geometry and hit target must still match (canvas/SVG too) |
| `wheel` | `{ref?, x?, y?, deltaX?, deltaY?}` integers ±20000, one nonzero | Native wheel, node/frame guard; reports offsets. macOS without verified geometry: `scroll` with the pane ref |
| `highlight` | `{ref, clear?}` | 1500 ms outline, no focus or input |

Native date fields take `YYYY-MM-DD`; text datepickers the page's displayed format. Reads show editor text, validity and validation messages.

Native input needs the Vibe Work window in front; it never brings itself forward. Behind another app, minimized or in the tray, nothing is sent: ask the user to bring it forward, read the page, then retry.

Text entry, `insert_text` and page keys use the native keyboard path with trusted events and a settled read-back. A changed document or focus refuses input; an interrupted dispatched operation is never retried. Read the page after a failed or unknown result before another action. An input with an unresolved native receipt cannot be repeated by the agent in the same document, also after reading again. Native select popups and date/colour picker keys give no receipt: use `select` or the field's value action. Clipboard shortcuts refuse because they bypass the protected clipboard; use `clipboard.read`/`clipboard.write` and `insert_text` instead. Copy/cut/paste authority remains Full only.

## Reading options (`browser_read`)

| Option | Rule |
|---|---|
| `selector: {kind: "css"\|"xpath"\|"alt"\|"title"\|"testId", value, index?}` | Refs and `matches.count/complete`; exact attributes; CSS enters open shadow roots; XPath document semantics; `index`: one of the first 200 matches (zero-based, document then frames). Unreadable frames or a capped scan = incomplete. No `find` or cursors |
| `ref` | That retained subtree (open shadow roots); frames have their own refs; `textPage` pages it. A replaced root refuses with a fresh-read instruction |
| `extract: {kind: "table"\|"list"\|"links"}` + `saveTo: "csv"\|"xlsx"` | Saves rendered rows in the chat folder. `extract.selector` picks roots/items; list columns `{name, selector, attribute: "text"\|"href"}`; tables: positional columns; `extract.index`: table across frames. Receipt: files, row counts, sources, coverage. `appendFrom` adds pages to a collection, `deduplicate: true`; each export makes new files. CSV formula-like cells are text; spreadsheet cells contain no formulas |

Selector queries match a redacted DOM snapshot. Protected fields expose identity, label and state attributes; their other attributes and editor text are unavailable to predicates. Secret values and hidden text are absent; interaction-state CSS pseudo-classes are unavailable. Extraction excludes secret editor text, including when a selected parent contains it.

## Dialogs, sign-in and downloads

| Action | Arguments | Rule |
|---|---|---|
| `page.dialogs` | | The held alert, confirm, prompt, leave-page or HTTP sign-in dialog, also while page JavaScript is blocked. A click or navigation that opens a dialog returns it; answer it and read the result before repeating the input |
| `page.dialog.accept`, `page.dialog.dismiss`, `page.dialog.prompt` | `{dialogId}`; prompt `{dialogId, text}` | Stop dismisses held dialogs. Reading and dismissing run in Ask; accepting a dialog and entering prompt text ask |
| `auth.basic` | `{url, login, dialogId?}` (vault login name or id) | HTTP Basic sign-in with a browser approval card below Full access and the existing separate vault approval; checks the challenge origin; credentials never appear in the tool. Read `page.dialogs` after a refusal. If the login is missing, ask the user to save or choose it in the vault. Never supply credentials as text or URL arguments |
| `files.download` | `{url}` | HTTP(S) with the profile's cookies; returns `downloadId`. Click receipts list new `untrustedPageContent.downloads` |
| `files.wait_download` | `{downloadId?, timeoutMs?}` | ≤20 s (default); `timedOut` = unfinished. Omit the id only when this chat has exactly one download |
| `files.list_downloads`, `files.save_download` | save `{downloadId}` | This chat's recent downloads. Completed files are saved automatically to the chat's `downloads/` folder; save returns the path. A failed/interrupted download has no complete artifact. Wait before using a file |

Ask: ordinary GET downloads need no approval; saving with `files.save_download` asks. Listing and waiting run.

## Windows and screenshots

| Action | Arguments | Rule |
|---|---|---|
| `window.open` | `{url?}` | New window with one page for this chat; receipt `windowId`, `tab` |
| `window.focus`, `window.close` | `{windowId}` | Window actions run in Ask, retain Stop and ownership checks, and never target another chat. The engine limits live windows; close one to free a slot |
| `window.list` | | Focus, visibility, size of this chat's windows |
| `window.resize` | `{windowId, width, height}` | 400–4096 × 300–4096 content px; main app window ids refuse |
| `capture.screenshot` | `{ref?}` or `{clip?: {x, y, width, height}}` (CSS px) | PNG of one visible element or rectangle; neither = viewport. Scroll offscreen elements into view first. The usual vault screenshot restrictions apply |

`file:` navigation: one PDF, HTML or image from the chat's first folder, ≤32 MiB. The built-in browser grants the selected document only; other local paths and linked local resources remain unavailable. `document_read` reads a saved PDF when the viewer shows no text.

## Batches and hand-over

`browser_act {steps: [...]}` runs 1–16 ordinary action objects in order. Each step uses the same access tier, consent, target binding and Stop checks as a single action. It returns ordered actual receipts and stops on a refusal, unmet condition, unknown outcome, or execution/output budget. Site-secret reads are checked against Full access before the first step. Batches cannot nest; remaining steps are not automatically retried.

An open user question hands this chat's browser back: capture and agent actions wait, the page stays open, and a confirmed answer permits the next browser action. Skip keeps browser work held until a new user message. Stop cancels browser work; a late answer cannot resume it.
