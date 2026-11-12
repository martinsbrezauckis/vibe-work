# Your Chrome/Edge/Firefox/Safari: more actions

Use with `chrome_execute {sessionId,action,args}`; `chrome_status {action}` returns each schema. Access modes, tab ownership and Stop apply as in the skill.

## Support by browser

`chrome_status` lists each profile's actions; an action it does not list is refused for that browser, so choose another action or browser instead of retrying. Chrome and Edge share one action set.

- Chrome/Edge only: `bookmarks.*`, `history.*`, `tabs.group`/`tabs.group_update`/`tabs.ungroup`, `network.intercept`, `files.drop`, `webmcp.*`, `extensions.*`, `notifications.*`.
- Firefox, Safari and the built-in browser, not Chrome/Edge: `page.read {selector}` (find by CSS, XPath, alt, title or test id), `page.select {values}` (multiple select), `page.reload {bypassCache}`, `page.inspect`, `page.errors.clear`. In Chrome/Edge use `page.find` with a label, role or text.
- Firefox has no `page.double_click`, `page.triple_click`, `tabs.pin`, tab groups, `tabs.audio`/`tabs.mute`, bookmarks, history, `files.drop` or `network.intercept`.
- Safari has no page dialogs (`page.dialogs`, `page.dialog.*`), `auth.basic`, downloads (`files.download`, `files.wait_download`, `files.list_downloads`, `files.save_pdf`), `network.headers` or `network.body`, besides the Chrome/Edge-only list.
- No browser has continuous page video: `recording.video.start/stop` report unavailable; `recording.start/stop` make an action-screenshot GIF without audio.

Every connected browser has harder forms (`page.type`, `page.select`, `page.check`/`page.uncheck`, `page.commit`), waits (`page.wait`, `page.wait_for`), table/list/link export from `page.read` to CSV or xlsx, media control (`media.*`), `page.highlight`, rich clipboard and page checks (`console.read`, `page.errors`, `network.list`, `tabs.emulate` sizes; light/dark `colorScheme` in Chrome/Edge and Firefox). Navigation (`page.back`, `page.forward`, `page.reload`), `page.right_click`, `page.middle_click`, `clipboard.write` and `console.clear` run in every connected browser too. The limits below say where a browser differs.

## Profiles, tabs and windows

Each chat has one fixed browser profile. A single eligible profile or the user's default seeds it; several profiles without a default require the user's choice in the chat. `chrome_status` includes `selectedProfile` and named `profileChoices` with connection states. `chrome_session open` and action receipts include the actual `profileId` and `profileName`. Chrome and Edge IDs with the same UUID are separate accounts. A profile rename changes its label, never its identity. An unavailable selected profile runs no action; select a connected profile or reconnect it. A switch made by the user releases the old task first; an unconfirmed release keeps work stopped. First pairing and a changed owner require the extension's approval ceremony; only approved identities reconnect automatically, including after an app or extension update.

`tabs.context {}` lists open tab titles/addresses without handing over a window; `{tabId}` reads its bounded page text. Other chats' tabs are excluded; no activation or ownership change. Firefox sees only its connected agent window, not separate personal profiles.

`tabs.pin {pinned:true|false,tabId?}` pins/unpins a tab in this chat’s owned window; receipt reads real pinned state and met. Requires negotiated support; personal tabs outside the owned window are refused.

`window.close {windowId?}` closes this chat’s exact owned-window snapshot in Chrome, Edge, Firefox or Safari (at most 32 tabs in Chrome/Edge/Safari; 128 in Firefox). It never closes personal or other-chat tabs. The final inventory returns `windowClosed`, `allOwnedTabsClosed`, `closedTabs`, `skippedTabs`, `remainingTabs` and reasons. `met` means the window closed; personal, moved, changed and newly arrived tabs are preserved. An empty window that remains open reports windowClosed:false. Missing confirmation returns an unknown outcome: inspect the current tabs before continuing, and do not repeat automatically.

`window.focus {tabId?}` focuses the chat’s owned window without changing its active tab; the receipt returns focused and met.

`tabs.group {tabId?,groupId?,title?}` creates or joins an owned group. `tabs.group_update {tabId?,title}` edits its observed group. Supplying a title or renaming a group asks for approval in Ask; Accept edits and Full access include these writes. Grouping without a title remains an ordinary browsing action. Every tab in the window must belong to this chat. `tabs.ungroup {tabId?}` finishes after a fresh inventory confirms membership; missing readback is unknown.

`windows.list` reads the current chat window’s native focus and geometry.

Deleting a browser chat ends control and leaves its window and tabs open. The result distinguishes `controlEnded` from `windowClosed`; individual owned-tab close remains available separately.

A completed turn retains the same session for follow-ups after acknowledged visual release. Stop, chat close, app exit or 30 minutes without browser work ends the session. An unanswered user question holds that expiry until answered or the chat ends. Stop and idle expiry leave tabs open.

A verified popup receipt retains its exact opener. `popups.closedCount` counts confirmed disappearance once, including an unselected popup; `openCount` is unknown until inventory is complete. `trackingTruncated` reports when the bounded retained set is full. When that popup closes, `popupContinuation` reports whether the same opener document, origin and window are still available. A confirmed return updates the session target; read that page before sending more input. A changed, closed, moved or other-chat opener is unavailable, and no unrelated tab is selected. Stop, a new selection or a permission change cancels a pending return.

## Hand-over to the user

An open user question hands this chat's browser back: capture and agent actions wait, the page stays open, and a confirmed answer permits the next browser action. Skip keeps browser work held until a new user message. Stop cancels browser work; a late answer cannot resume it. Use this for steps only the user can do (CAPTCHA, payment, passkey, a code sent to the user), then read the page before continuing.

## Input

`page.resolve_point` returns a short-lived pointId for a visible node in an owned Chrome, Edge or connected Firefox tab. `page.move` and `page.wheel` use that pointId, recheck the same node and geometry, and report the observed native event and actual scroll offsets. Wheel accepts deltaX/deltaY in pixels. Covered or changed points refuse before input; points inside child frames use existing frame element actions.

`page.focus`, `page.blur` and `page.insert_text` use pointId in Chrome/Edge and an observed ref in Firefox and Safari. Insertion replaces the current selection, preserves surrounding text and editable markup, sends beforeinput/input without key events, and reports actual focus/value. Sensitive, replaced or canceled fields return a refusal or met:false with a reason.

Page reads withhold the values of protected fields (passwords, one-time codes, card numbers, masked inputs); the user fills them or `auth.fill` uses a vault login.

`page.wait_for` polls one CSS selector for attached, detached, visible or hidden. A hidden element remains attached. Conditions are read-only and bounded by timeoutMs (up to 20000); timeout returns met:false, and ambiguous or invalid selectors return a reason. `page.wait {networkIdle}` is Chrome/Edge only.

`page.scroll_into_view {selector,tabId?}` brings one unique main-document element into view and returns actual bounds and scroll-container offsets. Hidden or ambiguous selectors do not scroll.

`page.highlight` uses a checked pointId in Chrome/Edge or an observed ref in Firefox and Safari. It outlines the target for 1500 ms without focus or input; clear:true removes it.

`page.key` accepts page keys and editing shortcuts in every browser. Browser/system chords (including Cmd+Q/W/T/N/Tab/Space/H/M, Ctrl+Cmd+Q, Alt+Tab, browser reload/location and function keys) refuse before input; use a named browser action or ask the user. Safari native editing chords require a focused editable field, so Undo cannot fall through to a browser menu command. Copy/cut/paste retain Full-only clipboard authority.
Private-use function-key characters also refuse; supply the supported named page key instead.

## Clipboard, site data and retries

`clipboard.read_rich` and `clipboard.write_rich` require Full access in every browser. They take mime text/html or image/png; writes take text for HTML or base64 for PNG. Writes return actual clipboard content read back, including whether its bytes match the input. PNG reads return an image. Content is bounded to 256 KiB, HTML input to 60000 bytes; temporary transfer files remain private to the operation and are removed at completion. The chat mode itself must be Full access because writes also read clipboard content back; a write approval in a lower mode cannot authorize that readback.

`storage.change {operation:"clear",area:"local"|"session",tabId,origin}`: Full access only, exact HTTP(S) origin, in every browser. It rechecks after 300 ms and returns `before`, `remaining` and `cleared` counts, no stored values; repopulated storage is failed, missing read-back is unknown. Stop, permission changes and origin changes refuse before mutation. Local clearing leaves session storage intact; session clearing leaves other tabs' session stores intact. Other site stores are not included. Inspect a failed/unknown receipt before continuing; do not repeat a clear automatically.

Transport retries with the same authenticated browser call identity reuse its one-use decision or return the original result marked `requestReplay`. A changed input under that identity is refused. Interrupted requests and evicted results never dispatch again; a deliberate new action has a new call identity. Cached site-secret results still require the current Full access mode.

## Chrome/Edge diagnostics

`extensions.status {tabId}`: version/state, missing permissions/file-URL grant; `chrome_status`: off/disconnected. `extensions.list {tabId}`: others’ version/enabled. `notifications.create {tabId,id,title,message}`, `notifications.list {tabId}`, `notifications.clear {tabId,id}`: chat notifications, verified IDs; create/clear: Ask card; diagnostics: all modes.

## Your Firefox

Your Firefox is Vibe Work's own Firefox window: the user opens it with Settings > Browser > Your Firefox > Open window, and the agent works only in that window. If it is closed or not connected, ask the user to open it there. The window can stay open after Vibe Work quits. If Open window fails, ask the user to close the old Firefox window and open it again. On Linux the Snap Firefox cannot start this window: ask the user to install Mozilla's Firefox (the .deb package or the download). Each Open window starts with an empty Firefox profile: no site stays signed in from an earlier window, so sign in again with a vault login or hand the step to the user.

Frames work: frame refs and `frameId` reach ordinary web frames. Sandboxed, `data:` and other opaque frames are refused with a reason; open their content in an owned tab instead.

Firefox click and key receipts report newly loaded popups with effect:new_tab and tab IDs while retaining the original target. A popup observed opening then closing before load returns effect:popup_closed; existing or unrelated tabs are not claimed. Pages' popups are not yet owned by this chat: to work in one, open its address in an owned tab or ask the user.

Firefox on macOS uses a checked DOM route for `page.select`; receipts identify synthetic input/change events and verify the selected value. `page.click` with Control uses the page context-menu button on macOS.

Firefox `window.focus` activates the one currently visible tab in this chat’s owned window and reads back native focus. If the selected tab cannot be identified while hidden/minimized, it refuses without selecting another tab.

Firefox observed-ref clicks, typing, selection, pointer actions and focus controls wait up to two seconds for the same node to be visible, uncovered, enabled and geometrically stable before input. Stop, detachment or document change ends the wait; timeout sends no input. Coordinate-only actions keep their immediate point checks.

Firefox `page.read` accepts `selector:{kind,value,index?}`: kind is css, xpath, alt, title or testId; index is zero-based. `selectorCount` counts matches before indexing/paging. CSS and attribute selectors include observed closed shadow roots; XPath follows document semantics. The selected frame bounds the search. Unreadable frames and scan limits are explicit in coverage.

Firefox `page.select {ref,values:[...]}` sets the complete enabled option set of a native multiple select; an empty array clears it. Readback returns actual selected values and `trustedEvents:false`. Other native pointer/key input is browser-trusted; focus/blur/insert and checked DOM setters identify their synthetic events. Rich-editor refs, ARIA options, validation messages and image/reset input names appear in reads. Submission receipts distinguish observed validation, page handling and navigation; they do not establish server acceptance.

Firefox `page.drag` accepts a source ref or x/y and `path:[{x,y,durationMs?}]`, plus optional holdMs and button. Coordinates are top-frame viewport CSS pixels. Limits: 32 points, 250 ms per segment (default 50), 2000 ms hold, 4000 ms total requested duration. `path:[]` with positive holdMs performs one bounded hold/release. The result verifies native path events and release; no held state survives between calls. Stop releases the exact operation's input; native release can activate a click, so an interrupted result is unknown. HTML drag-data sources are unavailable through this route.

Firefox uses the same `window.close` contract above. Closing the last owned window leaves a separate neutral window to retain the connection. Network-idle waits remain unavailable.

Firefox `cookies.change {operation:"clear",tabId,origin}` requires Full access. It clears the selected host's applicable cookies across all paths in that tab's storage partition, including HttpOnly and parent-domain cookies, and reads the remaining count back. Unrelated hosts remain untouched; parent-domain cookies are shared by their matching subdomains. Counts contain no cookie values. Unsupported browsers do not acquire cookie clearing from a storage-clear request.

Firefox chats retain separate windows and references. A condition wait yields to other chats; handover and Stop release only their own chat. A question to the user holds agent actions until answered or a new message from the user arrives. Short-lived input marks show the agent's pointer or typing target, expire automatically and are omitted from page reads and captures. Changed references, documents and controller ownership return their reason and a recovery instruction; page changes alone do not identify who caused them.

## Your Safari

Safari's embedded extension follows the app release version. Pairing: the user clicks Pair or Replace soon after opening the pairing page; if pairing fails, ask the user to open the page again and click it right away.

Safari limits: frame ids are unusable, so read and act in the top document. Popups the page opens are not adopted; open the popup's address in an owned tab or ask the user. New tabs open inactive only (`tabs.create` gives `active:false`). Captures, screenshots and recordings work only on the active tab. A table export from a long table can stop early (17 of 41 rows in testing): compare the receipt's row count with the page and report missing rows. Safari's autoplay policy refuses `media.play`; pause, seek, mute and `media.read` work. A "Leave page?" sheet blocks the session and cannot be answered by the agent: ask the user to choose Leave Page in Safari. Native keys, shortcuts and pointer input need the Accessibility permission the user grants Vibe Work; keys are verified with the ABC and U.S. keyboard layouts.

Custom numeric datepicker formats come from the attached widget on Safari 18.4+; `dateFormatChecked` reports whether inspection ran. Native date/time/number/color/range setters, text regions and checkable controls return observed values/states; Safari DOM input is synthetic; visible hit, enabled/inert state and stable geometry are checked before input. `page.scroll` accepts a current ref and returns observed pane movement and boundaries. `page.read {selector}` and `page.select {values}` follow the Firefox description above.

Safari `auth.fill` uses vault approval, a secure matching origin and a same-origin POST form; receipts contain no credential values. `capture.screenshot` with `clip` and `capture.zoom` use CSS viewport rectangles and require the exact active tab; clipped/offscreen regions are unavailable. Safari click/commit receipts include observed page state; type/select support changes/full observations with revisions and captured continuations. Safari reads support subtree refs, open shadow roots, viewport bounds, counts and captured continuations; frame omissions and capture limits appear in coverage.

Safari `console.read` and `page.errors` report bounded page-world messages since attachment, with timestamps and omissions; earlier messages, other frames and object properties are unavailable. Safari `network.list` reports retained completed Resource Timing metadata with coverage limits; methods, headers, bodies and in-flight requests are unavailable.

Safari waits support visible text/refs, hidden refs, URL/pattern and DOM readiness with a deadline; network-idle waits are unavailable. Safari `window.resize` affects the chat window; `tabs.emulate` supports width/height/reset through window resizing and reports actual viewport dimensions and minimum-size mismatches.

Safari’s Ask Vibe Work menu, used only by the user, shares website pages, links and images through `person.share.v1`; sharing requires website access and the original menu document identity from Safari 18.4 or later.

Upload: Safari: visible/hidden file-input ref or one unique top-document file input, at most 16 files / 32 MiB total; receipt reads the actual FileList. OS chooser and directory control are unavailable.

Recording: Safari records the exact active tab: 512×288 frames, 500 ms display duration, at most 60 seconds / 120 frames / 8 MiB. Failed or unconfirmed capture is reported; Stop discards unsaved buffers.

Safari `cookies.change {operation:"clear",tabId,origin}` requires Full access and complete website permission coverage. It clears matching host/domain cookies across all paths in the tab’s exact nonprivate store, then returns counts after settling. Cookies can be shared across subdomains and ports. Partial website grants refuse before deletion; ask the user to allow all websites or clear cookies in Safari. Remaining cookies produce failed; missing read-back produces unknown. Inspect the result before another action; do not automatically repeat a clear.

Safari shows a brief input marker without changing page focus. Capture removes it; handover to the user, Stop and chat close wait for its removal. The marker is visual feedback only and is excluded from page text.
