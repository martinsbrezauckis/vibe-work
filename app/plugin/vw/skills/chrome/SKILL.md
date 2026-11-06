---
name: chrome
description: The person's own Chrome or Edge, visible in their window.
---

Store serves **0.1.5**; installs still on **0.1.4** update by themselves. Both are supported; app versions are independent. `session_status` reports installed `extensionVersion`; `updateNeeded` means below the supported minimum. New capabilities require reported support.

Chrome and Edge share the extension and permissions. Setup: Settings > Browser > Your Chrome / Your Edge.
`session_status` gives connection state and the person's next step. `addon_suggest({addon:"chrome",reason:"Use your signed-in browser"})` offers Your Chrome; `addon:"edge"` offers Your Edge. In chat, Connect Chrome starts setup; Connect Edge opens Settings > Browser > Your Edge, whose initial button is Set up…. Only the person installs, pairs or turns it on.
Beside Add a browser tab… in the composer, one dot for Chrome and one for Edge: green Ready, orange Extension not connected, red Browser not installed. Clicking opens the tab picker when one is ready, otherwise Settings > Browser.
`session_status`'s `chromeExtensionStore` is the last accepted release manifest's store state: `review` means in Google's review, `available` means listed, `null` means unknown.
Subagents cannot use Your Chrome or Your Edge; their browser tools are `browser_read` / `browser_act`.

`chrome_status {}`: connected profiles/actions; `allowedInThisMode` marks chat permissions. Optional `profileId` selects a browser's status, including `edge` or `Your Edge`. Ask-mode history and bookmark reads also report `approvalRequired`.
`{action}`: connected-profile schema in `inputSchemaJson` (compact JSON).
`{actions:[...]}`: up to 8 compact schemas with constraints, no descriptions.
Args by extension capability: 0.1.4 has `interactiveOnly`, `offset`, `limit`/`maxBytes` up to 500/64000; `includeText`, viewport/bounds and revisioned observations need 0.1.5.

`chrome_session {action:"open",profileId}` opens this chat's session; `get` reads, `cancel` cancels an action, `stop` ends work but leaves the window open.
Without `tabId`, no web page is selected. Empty `scope.tabIds` means no owned tab; `tabs.create` opens one.
Chat window titles need extension 0.1.5 (`session.title`); 0.1.4 opens without a title. Vibe Work supplies the title; tool arguments cannot set it.
A session ends with the turn; this chat's next use of its id reopens its window/tabs (`sessionReopened`); earlier turns' refs are stale.
The person's Stop in Chrome refuses actions for the rest of the turn.
`tabs.list` omits only other chats' tabs. Non-web pages (`about:blank`, New Tab) have `controllable: false`; page actions cannot navigate them. `tabs.create {url}` opens an HTTP/HTTPS page in a chat-owned tab.
`active` is per window; 0.1.4's `selection.activeTabIds` lists active tabs; `focusedWindowKnown:false`/`currentTabId:null` mean no focus report.

`page.read`: up to `maxChars` characters (default 800), `limit` refs (default 8), in receipt `maxBytes` (default 2000); `textOffset`/`offset` continue.
Inline reads cap `maxChars` and `maxBytes` at 4000; file exports use extension limits. Continuations name the action and args; an app continuation takes only its `cursor`. A single `page.read` with top-level `saveTo:"file"` saves that receipt in the chat folder (Accept edits or Full access); omitted or `"inline"` returns it inline. Saving does not imply complete page coverage.
`chrome_execute {sessionId,action,args}` returns observed receipts; page data: `untrustedPageContent`. Stale refs are refused; unknown outcomes are not auto-replayed.
`page.type` without `ref` types into the focused field in that tab (requires `page.type.focused`).
`page.scroll` accepts `direction` (`up`, `down`, `left`, `right`) for 600 pixels, or `x`/`y` pixel deltas, not both.
Click/hover/drag/drop: `ref` or `x`/`y`: the tab's viewport position in CSS pixels, also in frames; use coordinates for text/images/boxes without refs.
Batch `{sessionId,steps:[{action,args},...]}`: up to 16 steps/25 s; per-step mode checks and receipts; stops on refusal/failure/unknown. `nextStep`: first undispatched step when observation time is low.

In Ask, `history.read` and `bookmarks.read` offer a permission card: allow once, allow the same read for this session, or No. Session consent covers the same browser session, action and exact arguments; other actions keep their mode requirements.
Local-network pages: user must enable Settings > Developer > Local network pages for the agent; off: named sites over https only (`HTTPS_REQUIRED` otherwise). Link-local, cloud metadata and other apps' local ports always refused.
With a supporting extension, `clipboard.read`/`clipboard.write` read and write clipboard text; `tabs.emulate` sets device size, colour scheme and touch; `network.intercept` supplies test responses.
Confirmed files: chat folder. `files.wait_download` waits for this chat's attributed download; `finished` reports completion, `timedOut` an unfinished wait. Ambiguous downloads are refused. `files.list_downloads` lists Chrome saves, not necessarily this chat's.
Uploads: chat-relative `paths` or selected `fileIds`. Vault sign-in: `auth.fill {ref,login}`/`auth.basic {url,login}`; no password arguments; separate consent.
