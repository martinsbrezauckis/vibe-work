# Your Chrome/Edge/Firefox/Safari: more actions

Use with `chrome_execute {sessionId,action,args}`; `chrome_status {action}` returns each schema. Access modes, tab ownership and Stop apply as in the skill. Firefox details: [firefox.md](firefox.md); Safari details: [safari.md](safari.md).

## Support by browser

`chrome_status` lists each profile's actions; an action it does not list is refused for that browser, so choose another action or browser instead of retrying. Chrome and Edge share one action set.

| Where | Actions |
|---|---|
| Every connected browser | `page.type`, `page.select`, `page.check`/`page.uncheck`, `page.commit`, `page.wait`, `page.wait_for`, `page.read` table/list/link export to CSV or xlsx, `media.*`, `page.highlight`, rich clipboard, `console.read`, `console.clear`, `page.errors`, `network.list`, `tabs.emulate` sizes, `page.back`/`forward`/`reload`, `page.right_click`, `page.middle_click`, `clipboard.write` |
| Chrome/Edge only | `bookmarks.*`, `history.*`, `tabs.group`/`tabs.group_update`/`tabs.ungroup`, `network.intercept`, `files.drop`, `webmcp.*`, `extensions.*`, `notifications.*`, `page.wait {networkIdle}`; `tabs.emulate` `colorScheme` light/dark also in Firefox |
| Firefox, Safari, built-in; Chrome/Edge when listed (newer extension) | `page.read {selector}` (CSS, XPath, alt, title or test id), `page.select {values}` (multiple select), `page.reload {bypassCache}`, `page.inspect`, `page.errors.clear`, `page.drag {path, holdMs, button}` (pointer path, hold, long press), `cookies.change {operation:"clear"}` (Full access). Otherwise use `page.find` with a label, role or text; an older extension answers EXTENSION_UPDATE_REQUIRED |
| Not in Firefox | `page.double_click`, `page.triple_click`, `tabs.pin`, tab groups, `tabs.audio`/`tabs.mute`, bookmarks, history, `files.drop`, `network.intercept` |
| Not in Safari | page dialogs (`page.dialogs`, `page.dialog.*`), `auth.basic`, downloads (`files.download`, `files.wait_download`, `files.list_downloads`, `files.save_pdf`), `network.headers`, `network.body`, and the Chrome/Edge-only row |
| Chrome/Edge when listed | `recording.video.start {tabId, maxSeconds}`: this chat's own tab, WebM, no audio, ≤300 s (default 60); `recording.video.stop {filename}` saves in the chat folder, `{discard:true}` drops it; the user's Stop discards it. The file lasts from start to stop (≤ maxSeconds); unchanged or hidden time holds the last picture. Elsewhere unavailable; `recording.start/stop` make an action-screenshot GIF without audio |

## Profiles and session

Each chat has one fixed browser profile. A single eligible profile or the user's default seeds it; several profiles without a default require the user's choice in the chat. `chrome_status`: `selectedProfile`, named `profileChoices` with connection states; `chrome_session open` and receipts: actual `profileId`, `profileName`. Chrome and Edge IDs with the same UUID are separate accounts; a rename changes the label, never the identity. An unavailable selected profile runs no action; select a connected profile or reconnect it. A switch made by the user releases the old task first; an unconfirmed release keeps work stopped. First pairing and a changed owner require the extension's approval ceremony; only approved identities reconnect automatically, including after an app or extension update.

A completed turn retains the same session for follow-ups after acknowledged visual release. Stop, chat close, app exit or 30 minutes without browser work ends the session. An unanswered user question holds that expiry until answered or the chat ends. Stop and idle expiry leave tabs open. Deleting a browser chat ends control and leaves its window and tabs open (`controlEnded` vs `windowClosed`).

## Hand-over to the user

An open user question hands this chat's browser back: capture and agent actions wait, the page stays open, and a confirmed answer permits the next browser action. Skip keeps browser work held until a new user message. Stop cancels browser work; a late answer cannot resume it. Use this for steps only the user can do (CAPTCHA, payment, passkey, a code sent to the user), then read the page before continuing.

## Tabs and windows

| Action | Arguments | Rule |
|---|---|---|
| `tabs.context` | `{}` or `{tabId}` | `{}`: open tab titles/addresses without handing over a window; `{tabId}`: its bounded page text. Other chats' tabs are excluded; no activation or ownership change. Firefox sees only its agent window |
| `tabs.pin` | `{pinned, tabId?}` | Owned window only, negotiated support; receipt reads real pinned state and met. Personal tabs outside the owned window are refused |
| `tabs.group` / `tabs.group_update` / `tabs.ungroup` | `{tabId?, groupId?, title?}` / `{tabId?, title}` / `{tabId?}` | Supplying a title or renaming a group asks for approval in Ask; Accept edits and Full access include these writes. Grouping without a title remains an ordinary browsing action. Every tab in the window must belong to this chat. Ungroup finishes after a fresh inventory; missing readback is unknown |
| `tabs.create` | `{url, active?}` | Chrome/Edge: an active created tab (the default) becomes the session's tab; `active:false` only when it has none |
| `window.focus` | `{tabId?}` | Focuses the owned window without changing its active tab; receipt: focused, met |
| `windows.list` | | Current chat window's native focus and geometry |
| `window.close` | `{windowId?}` | Closes this chat's exact owned-window snapshot (≤32 tabs Chrome/Edge/Safari, 128 Firefox). It never closes personal or other-chat tabs. Receipt: `windowClosed`, `allOwnedTabsClosed`, `closedTabs`, `skippedTabs`, `remainingTabs`, reasons; `met` = window closed; personal, moved, changed and new tabs are kept; an empty window left open is windowClosed:false. Missing confirmation returns an unknown outcome: inspect the current tabs before continuing, and do not repeat automatically |

Popups: a verified popup receipt retains its exact opener; `popups.closedCount` counts confirmed closes once, `openCount` is unknown until inventory is complete, `trackingTruncated` marks a full retained set. When the popup closes, `popupContinuation` says whether the same opener document, origin and window remain; a confirmed return updates the session target: read that page before sending more input. A changed, closed, moved or other-chat opener is unavailable, and no unrelated tab is selected. Stop, a new selection or a permission change cancels a pending return.

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

Page reads withhold the values of protected fields (passwords, card numbers, security codes and expiry dates, masked inputs), also when they are labelled in another language; the user fills them or `auth.fill` uses a vault login. One-time codes are ordinary fields: read them for the user. In Chrome and Edge, typing into a field the page marks as a one-time code needs Full access, and a field masked only by the page's styling is not recognised.

## Clipboard, site data and retries

`clipboard.read_rich {mime}` / `clipboard.write_rich {mime, text|base64}` require Full access in every browser. MIME text/html (text, ≤60000 bytes) or image/png (base64); ≤256 KiB. Writes return the clipboard read back and whether the bytes match; PNG reads return an image; temporary transfer files remain private to the operation and are removed at completion. The chat mode itself must be Full access because writes also read clipboard content back; a write approval in a lower mode cannot authorize that readback.

`storage.change {operation:"clear",area:"local"|"session",tabId,origin}`: Full access only, exact HTTP(S) origin, in every browser. Rechecks after 300 ms; returns `before`, `remaining`, `cleared` counts, no values; repopulated = failed, missing read-back = unknown. Stop, permission changes and origin changes refuse before mutation. Local clearing keeps session storage; session clearing keeps other tabs' stores; other site stores are not included. Inspect a failed/unknown receipt before continuing; do not repeat a clear automatically.

Transport retries with the same authenticated browser call identity reuse its one-use decision or return the original result marked `requestReplay`. A changed input under that identity is refused. Interrupted requests and evicted results never dispatch again; a deliberate new action has a new call identity. Cached site-secret results still require the current Full access mode.

## Chrome/Edge

- Newer extension (only when `chrome_status` lists it), same arguments as Firefox: `page.read {selector:{kind,value,index?}}` (kind css/xpath/alt/title/testId, zero-based index; open shadow roots only); `page.select {ref,values:[...]}` (`[]` clears; `trustedInput:false`); `page.type` into date/time/month/week/datetime-local part by part with trusted keys (`page.type.dateKeys`, `route:"keyboard"`; colour fields and clearing use the value setter, `trustedInput:false`); `page.drag {ref|x,y, path:[{x,y,durationMs?}], holdMs?, button?}` (top document or same-process frame; a held button is always released); `page.reload {bypassCache}`; `page.inspect`; `page.errors.clear`; `cookies.change {operation:"clear"}` (all paths, parent-domain and partitioned cookies of the host). Ref clicks wait ≤2 s for an enabled, visible, stable, uncovered target. A popup closed by the agent's input = `effect:"tab_closed"`. `tabs.create {reopenTabId}` reopens a tab this chat closed with `tabs.close_owned`, once, as a new owned tab at its last address, no history; local, intranet and file addresses are refused. Protected fields (passwords, PINs, payment cards, masked fields) read as `[withheld]` with `valueWithheld`.
- Diagnostics: `extensions.status {tabId}` (version/state, missing permissions/file-URL grant; `chrome_status` shows off/disconnected), `extensions.list {tabId}` (others' version/enabled). `notifications.create {tabId,id,title,message}`, `notifications.list {tabId}`, `notifications.clear {tabId,id}`: chat notifications, verified IDs; create/clear: Ask card; diagnostics: all modes.
- `capture.screenshot {labels:true}`: up to 12 visible controls labelled on a PNG viewport copy; numbers map to executable refs; locations verified before and after; page unchanged; inline only.
- Every browser with PNG viewport capture: `capture.screenshot {compare:"before"}` stores a session/tab baseline; `compare:"after"` returns the image, changed-pixel percentage and pixel bounds. Viewport, scale and origin must match; exact RGBA changes count, animations included; ≤8 baselines.
- `page.read` export: table exports need `args.ref` for one table subtree; `args.interactiveOnly:false` exposes table/list structure; the returned cursor continues captured rows; `extract.header:false` keeps a continuation's first data row. Table/list/link records with positional column names; CSS and relative-column queries are built-in-only. Saved rows cover the returned read and its coverage limits.
- `page.add_style {css, tabId?}` (all four browsers) uses Full access and the same document/vault exclusion as `raw.execute`; result: accepted rule count. `console.clear {tabId?}` clears this tab's retained-console view; later messages stay readable, `page.errors` is separate, filters and paging stay.
