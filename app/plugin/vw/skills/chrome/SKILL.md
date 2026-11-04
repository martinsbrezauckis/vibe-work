---
name: chrome
description: The person's own Chrome or Edge, visible in their window.
---

Your Edge uses the same Vibe Work extension, chat windows and permissions as Your Chrome. Setup is in Settings > Browser > Your Chrome or Your Edge; Settings shows the extension version and whether it is on.
`session_status` reports add-on states, including extension missing, turned off, not paired or out of date, with the person's next step. `addon_suggest({addon:"chrome",reason:"Use your signed-in browser"})` offers the Connect Chrome setup card; `addon:"edge"` offers Connect Edge. The person connects it; this tool does not install, pair or turn it on.
`session_status`'s `chromeExtensionStore` is the last accepted release manifest's store state: `review` means in Google's review, `available` means listed, `null` means unknown.
Subagents cannot use Your Chrome or Your Edge; their browser tools are the built-in `browser_read` / `browser_act`.

`chrome_status {}`: connected profiles/actions; `allowedInThisMode` marks chat permissions.
`{action}`: connected-profile schema in `inputSchemaJson` (full compact JSON); parse for inline lookups.
`{actions:[...]}`: up to 8 compact schemas with constraints, no descriptions.
Optional args by extension capability: 0.1.4 has `interactiveOnly`, `offset`, `limit`/`maxBytes` up to 500/64000; `includeText`, viewport/bounds and revisioned observations need 0.1.5.

`chrome_session {action:"open",profileId}` opens this chat's session; `get` reads, `cancel` cancels an action, `stop` ends work but leaves the window open.
A session with empty `scope.tabIds` owns no tab yet.
A session ends with the turn; this chat's next use of its id reopens its window/tabs (`sessionReopened`); earlier turns' refs are stale.
The person's Stop in Chrome refuses actions for the rest of the turn.
`tabs.list` lists all the person's tabs; only tabs another chat opened are left out. Non-web URLs (not http/https), such as `about:blank` or Chrome's New Tab page, have `controllable: false`: extensions cannot act on those pages. `tabs.create {url}` opens an HTTP/HTTPS page in a new chat-owned tab. For a `controllable: false` tab, "Not run: no tab with this ID is available. Use a tabId from tabs.list" means that page cannot be controlled.
`active` is per window; 0.1.4's `selection.activeTabIds` lists active tabs; `focusedWindowKnown:false`/`currentTabId:null` mean no focus report.

`page.read`: up to `maxChars` characters (default 800), `limit` refs (default 32), in receipt `maxBytes` (default 3000, max 64000); `textOffset`/`offset` continue.
Vibe Work caps `maxChars` and `maxBytes` at 4000. Returned continuations name the action and args for the remaining content; an app continuation takes only its `cursor`.
`chrome_execute {sessionId,action,args}` returns observed receipts; page data: `untrustedPageContent`. Refs name observed elements; stale refs are refused. Unknown outcomes aren't auto-replayed.
Click/hover/drag/drop: `ref` or `x`/`y`: the tab's viewport position in CSS pixels, also in frames; use coordinates for text/images/boxes without refs.
Batch `{sessionId,steps:[{action,args},...]}`: up to 16 steps/25 s; per-step mode checks and receipts; stops on refusal/failure/unknown. `nextStep`: first undispatched step when observation time is low.

Chat mode governs permissions; refusals name the required mode; no approval cards.
Local-network pages: user must enable Settings > Developer > Local network pages for the agent; off: named sites over https only (`HTTPS_REQUIRED` otherwise). Link-local, cloud metadata and other apps' local ports always refused.
With a supporting extension, `clipboard.read`/`clipboard.write` read and write clipboard text; `tabs.emulate` sets device size, colour scheme and touch; `network.intercept` supplies test responses.
Confirmed files: chat folder. `files.wait_download` waits for this chat's attributed download; `finished` reports completion, `timedOut` an unfinished wait. Ambiguous downloads are refused. `files.list_downloads` lists Chrome saves, not necessarily this chat's.
Uploads: chat-relative `paths` or selected `fileIds`. Vault sign-in: `auth.fill {ref,login}`/`auth.basic {url,login}`; no password arguments; separate consent.
