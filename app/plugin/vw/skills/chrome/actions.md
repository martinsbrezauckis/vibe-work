# Your Chrome/Edge/Firefox/Safari: more actions

`chrome_execute {sessionId,action,args}` runs actions; `chrome_status {action}` gives schemas. Access modes, tab ownership and Stop apply as in the skill. Firefox: [firefox.md](firefox.md); Safari: [safari.md](safari.md).

## Support by browser

`chrome_status` lists each profile's actions and extension version; an action it does not list is refused for that browser, so choose another action or browser instead of retrying. Chrome and Edge can have different options while their installed extensions differ. Before using an optional field, look up `chrome_status {action,profileId}` and use its returned `inputSchemaJson`.

| Where | Actions |
|---|---|
| Every connected browser | `page.type`, `page.select`, `page.check`/`page.uncheck`, `page.commit`, `page.wait`, `page.wait_for`, `page.read` table/list/link export to CSV or xlsx, `media.*`, `page.highlight`, rich clipboard, `console.read`, `console.clear`, `page.errors`, `network.list`, `tabs.emulate` sizes, `page.back`/`forward`/`reload`, `page.right_click`, `page.middle_click`, `clipboard.write` |
| Chrome/Edge only | `bookmarks.*`, `history.*`, `tabs.group`/`tabs.group_update`/`tabs.ungroup`, `network.intercept`, `files.drop`, `webmcp.*`, `extensions.*`, `notifications.*`, `page.wait {networkIdle}`; `tabs.emulate` `colorScheme` light/dark also in Firefox |
| Firefox, Safari, built-in; Chrome/Edge when listed | `page.read {selector}` (CSS, XPath, alt, title or test id), `page.select {values}` (multiple select), `page.reload {bypassCache}`, `page.inspect`, `page.errors.clear`, `page.drag {path, holdMs, button}` (pointer path, hold, long press), `cookies.change {operation:"clear"}` (Full access). Otherwise use `page.find` with a label, role or text; an older extension answers EXTENSION_UPDATE_REQUIRED |
| Not in Firefox | `page.double_click`, `page.triple_click`, `tabs.pin`, tab groups, `tabs.audio`/`tabs.mute`, bookmarks, history, `files.drop`, `network.intercept` |
| Not in Safari | page dialogs (`page.dialogs`, `page.dialog.*`), `auth.basic`, downloads (`files.download`, `files.wait_download`, `files.list_downloads`, `files.save_pdf`), `network.headers`, `network.body`, and the Chrome/Edge-only row |
| Chrome/Edge/Firefox/Safari when listed | `recording.video.start {tabId, maxSeconds}`: this chat's own tab, VP8 WebM, no audio, ≤300 s (default 60), ≤1,200 frames. `recording.video.stop {filename}` saves in the chat folder; `{discard:true}` drops it. Start and stop in the same turn; the turn's end discards it. Protected frames end capture; Stop saves prior safe frames. Pause, the user's Stop, permission downgrade or encoder loss discards. Firefox/Safari use guarded screenshot frames. When video is unavailable, `recording.start/stop` make an action-screenshot GIF without audio |

## Profiles and browser sessions

Picked default until first use; other use shows a chat notice. One external profile/chat then locks until agent window close or Stop/release; built-in browser works in parallel. Sole eligible/default seeds; otherwise user picks. `chrome_status` lists `selectedProfile` and named `profileChoices`; receipts give `profileId` and `profileName`. Chrome and Edge IDs differ even with the same UUID; renames only labels. If unavailable, reconnect or choose a connected profile. Unconfirmed release blocks switching. Pairing and owner changes need extension approval; approved IDs reconnect after updates.

A completed turn retains its browser session after acknowledged visual release. Stop, chat close, app exit or 30 minutes idle ends it; an unanswered user question holds expiry. Stop, idle expiry and chat deletion leave tabs open (`controlEnded` versus `windowClosed`).

Pause holds this chat's browser calls. Resume or a new user message permits work after a fresh page read. In Chrome and Edge the user can also press Ctrl+Shift+9 (Pause/Resume) or Ctrl+Shift+8 (Stop), Command on macOS, changeable in the browser's extension shortcuts. Pages leave fullscreen while the agent works; Pause lets the user use fullscreen.

## Hand-over to the user

An open user question holds capture and agent actions while the page stays open. A confirmed answer resumes; Skip holds until a new user message; Stop cancels, and a late answer cannot resume. Use for sign-in challenges, passkeys, biometrics and CAPTCHAs, then reread the page.

## Tabs and windows

| Action | Arguments | Rule |
|---|---|---|
| `tabs.context` | `{}` or `{tabId}` | `{}`: open tab titles/addresses without handing over a window; `{tabId}`: its bounded page text. Other chats' tabs are excluded; no activation or ownership change. Firefox sees only its agent window |
| `tabs.inspect_other` | `{tabIds:[23,24]}` | Inspect explicitly listed tab IDs. First list tabs and pass the integer `tabIds` array; an individual `tabId` is not this action's argument |
| `tabs.pin` | `{pinned, tabId?}` | Owned window only, negotiated support; receipt reads real pinned state and met. Personal tabs outside the owned window are refused |
| `tabs.group` / `tabs.group_update` / `tabs.ungroup` | `{tabId?, groupId?, title?}` / `{tabId?, title}` / `{tabId?}` | Supplying a title or renaming a group asks for approval in Ask; Accept edits and Full access include these writes. Grouping without a title remains an ordinary browsing action. Every tab in the window must belong to this chat. Ungroup finishes after a fresh inventory; missing readback is unknown |
| `tabs.create` | `{url, active?}` | Chrome/Edge: a created tab (active by default) becomes the browser session's tab; `active:false` only if none |
| `window.focus` | `{tabId?}` | Focuses the owned window without changing its active tab; receipt: focused, met |
| `windows.list` | | Current chat window's native focus and geometry |
| `window.close` | `{windowId?}` | Closes this chat's exact owned-window snapshot (≤32 tabs Chrome/Edge/Safari, 128 Firefox). It never closes personal or other-chat tabs. Receipt: `windowClosed`, `allOwnedTabsClosed`, `closedTabs`, `skippedTabs`, `remainingTabs`, reasons; `met` = window closed; personal, moved, changed and new tabs are kept; an empty window left open is windowClosed:false. Missing confirmation returns an unknown outcome: inspect the current tabs before continuing, and do not repeat automatically |

Popup receipts retain the opener. `popups.closedCount` counts confirmed closes once; `openCount` stays unknown until inventory completes; `trackingTruncated` marks a full set. After close, `popupContinuation` confirms whether the same opener document, origin and window remain. A confirmed return updates the target: reread before input. Changed, closed, moved or other-chat openers are unavailable; no unrelated tab is selected. Stop, selection or permission change cancels a pending return.

Focus routes differ: `chrome_execute {sessionId,action:"window.focus",args:{tabId:23}}` uses a browser tab ID; the built-in browser's `browser_act {action:"window.focus",windowId:"..."}` uses its own window ID. Check the selected tool's schema before sending either.

## Input

| Action | Arguments | Rule |
|---|---|---|
| `page.resolve_point` | node | Short-lived `pointId` for a visible node (Chrome, Edge, Firefox) |
| `page.move`, `page.wheel` | `pointId`; wheel `deltaX`/`deltaY` px | Rechecks node and geometry; reports native event and scroll offsets. Covered or changed points refuse before input; child-frame points use frame element actions |
| `page.focus`, `page.blur`, `page.insert_text` | `pointId` (Chrome/Edge), ref (Firefox/Safari) | Insert replaces the selection, keeps surrounding text/markup, sends beforeinput/input without key events, reports focus/value. Sensitive, replaced or canceled fields return a refusal or met:false with a reason |
| `page.wait_for` | `selector`, state attached/detached/visible/hidden, `timeoutMs` ≤20000 | Read-only; hidden stays attached; timeout = met:false; ambiguous/invalid selector = reason |
| `page.scroll_into_view` | `{selector, tabId?}` | One unique main-document element; returns bounds and scroll offsets; hidden/ambiguous does not scroll |
| `page.highlight` | `pointId` (Chrome/Edge), ref (Firefox/Safari), `clear:true` | Outline for 1500 ms, no focus or input |
| `page.key` | key | Page keys and editing shortcuts in every browser. Browser/system chords (including Cmd+Q/W/T/N/Tab/Space/H/M, Ctrl+Cmd+Q, Alt+Tab, browser reload/location and function keys) refuse before input; use a named browser action or ask the user. Safari editing chords need a focused editable field. Copy/cut/paste retain Full-only clipboard authority. Private-use function-key characters also refuse; supply the supported named page key instead |

Page reads withhold the values of protected fields (passwords, card numbers, security codes and expiry dates, masked inputs), also when they are labelled in another language; the user fills them or `auth.fill` uses a vault login. Read and use one-time codes as ordinary values for the user. In Chrome and Edge, typing into a field the page marks as a one-time code needs Full access, and a field masked only by the page's styling is not recognised.

## Clipboard, site data and retries

`clipboard.read_rich {mime}` / `clipboard.write_rich {mime, text|base64}` require Full access in every browser. MIME text/html (text, ≤60000 bytes) or image/png (base64); ≤256 KiB. Writes return the clipboard read back and whether the bytes match; PNG reads return an image; temporary transfer files remain private to the operation and are removed at completion. The chat mode itself must be Full access because writes also read clipboard content back; a write approval in a lower mode cannot authorize that readback.

`storage.change {operation:"clear",area:"local"|"session",tabId,origin}`: Full access only, exact HTTP(S) origin, in every browser. Rechecks after 300 ms; returns `before`, `remaining`, `cleared` counts, no values; repopulated = failed, missing read-back = unknown. Stop, permission changes and origin changes refuse before mutation. Local clearing keeps session storage; session clearing keeps other tabs' stores; other site stores are not included. Inspect a failed/unknown receipt before continuing; do not repeat a clear automatically.

Transport retries with the same authenticated browser call identity reuse its one-use decision or return the original result marked `requestReplay`. A changed input under that identity is refused. Interrupted requests and evicted results never dispatch again; a deliberate new action has a new call identity. Cached site-secret results still require the current Full access mode.

## Chrome/Edge

- When `chrome_status` lists them: `page.read {selector:{kind,value,index?}}` (css/xpath/alt/title/testId, zero-based; open shadow roots); `page.select {ref,values:[...]}` (`[]` clears, synthetic input); `page.type` into date/time/month/week/datetime-local with trusted keys (`page.type.dateKeys`, `route:"keyboard"`); colour and clearing use a synthetic setter. `page.drag {ref|x,y,path:[{x,y,durationMs?}],holdMs?,button?}` releases the held button; same-process frames work. Also `page.reload {bypassCache}`, `page.inspect`, `page.errors.clear`, `cookies.change {operation:"clear"}` (all paths, parent-domain and partitioned cookies). `tabs.create {reopenTabId}` reopens one chat-closed tab without history; local, intranet and file addresses refuse.
- In every mode, protected fields yield `[withheld]` and `valueWithheld` and block scripts, `page.add_style`, `storage.read`, `cookies.read`, `console.read`, `page.errors`, `network.body` and unredacted `network.headers` (`protectedValuesWithheld` receipt); visible ones block capture/recording. This refusal is final: never hide fields, re-fetch the page or read app files/logs; tell the user. Use `page.read`, `page.type`, `page.click` or vault `auth.fill`.
- Diagnostics: `extensions.status {tabId}` (version/state, missing permissions/file-URL grant; `chrome_status` shows off/disconnected), `extensions.list {tabId}` (others' version/enabled). `notifications.create {tabId,id,title,message}`, `notifications.list {tabId}`, `notifications.clear {tabId,id}`: chat notifications, verified IDs; create/clear: Ask card; diagnostics: all modes.
- `capture.screenshot {labels:true}`: up to 12 visible controls labelled on a PNG viewport copy; numbers map to executable refs; locations verified before and after; page unchanged; inline only.
- PNG viewport capture: `capture.screenshot {compare:"before"}` stores a chat/tab baseline; `compare:"after"` returns the image, changed-pixel percentage and pixel bounds. Viewport, scale and origin must match; exact RGBA changes count, animations included; ≤8 baselines.
- `page.read` export: table exports need `args.ref` for one table subtree; `args.interactiveOnly:false` exposes table/list structure; the returned cursor continues captured rows. Put `extract:{header:false,...}` and `saveTo:"file"` at the top level of `chrome_execute`, not inside `args`; a continuation's first data row needs `extract.header:false`. Table/list/link records with positional column names; CSS and relative-column queries are built-in-only. Saved rows cover the returned read and its coverage limits.
- `page.add_style {css, tabId?}` (all four browsers) uses Full access and the same document/vault exclusion as `raw.execute`; result: accepted rule count. `console.clear {tabId?}` clears this tab's retained-console view; later messages stay readable, `page.errors` is separate, filters and paging stay.
