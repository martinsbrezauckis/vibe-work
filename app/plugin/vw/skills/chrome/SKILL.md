---
name: chrome
description: The user's own Chrome/Edge/Safari, or Vibe Work's own Firefox window.
---

## Setup
Extensions auto-update independently of the app. `session_status`: connection/next step, `extensionVersion`, `updateNeeded` (below minimum), support. `chromeExtensionStore`: `review`/`available`/`null` = reviewing/listed/unknown.
Connect Chrome/Edge separately; removing one keeps the other connected. `addon_suggest {addon:"chrome"/"edge",reason}`: Connect Chrome opens setup; Connect Edge opens Your Edge settings (Connect Edge). Only the user installs/pairs/enables. Add a browser tab… shows connection state; opens ready tab picker or Settings > Browser.
Safari (macOS): Settings > Browser > Your Safari switch/pairing; exact `safari:` profileId; no launch/`addon_suggest`. Firefox: `chrome_session` open with `firefox:agent` automatically starts its empty-profile window if on; only the user enables it. Subagents: `browser_read`/`browser_act` only.

## Browser sessions and tabs
`chrome_status {}`: profiles/actions, `allowedInThisMode`, `approvalRequired`; `profileId` filters (`edge`/`Your Edge`). Check `extensionVersion`/`features`. `chrome_status {action,profileId}` gives its `inputSchemaJson`; `{actions:[...]}` gives ≤8 schemas/constraints. Refresh after switches. `interactiveOnly`, `offset`, `limit`/`maxBytes` ≤500/64000, `includeText`, bounds/revisions: per support.
`chrome_session {action:"open",profileId}` opens; `get` reads; `cancel` cancels; `stop` ends work, keeps window. `close_browser {profileId}` (exact) frees a stuck chat before switching. Firefox: closes the app's window. Chrome/Edge/Safari: closes this chat's tabs, then Stop; user tabs stay. Ask: card; Accept edits/Full access: runs. Closed paired Chrome/Edge: `profileId:"chrome"`/`"edge"`/exact ID launches; ≤10 s, no Ask card. Reconnect: exact ID. Tabs persist.
Picked default until first use; other use shows a chat notice. One external profile/chat then locks until agent window close or Stop/release; built-in browser works in parallel. `browserTab`: `profileId,tabId`; open: `tabId` for a tab. No `tabId`: no page (`scope.tabIds:[]`); `tabs.create {url}`: HTTP/HTTPS. Title: `session.title`. Completed turns release control after acknowledgement; the browser session ID remains, refs expire. Stop blocks this turn.
CAPTCHA/payment/passkeys: `ask_user_question` hands over this chat's browser; read after the answer.
`tabs.list`: owned/handed windows, including blank/internal tabs. `tabs.close_user`: any listed tab; `tabs.close_owned`: chat-opened tabs. Non-web actions refuse. Handed-window tabs stay open until closed. `active`: per window; `selection.activeTabIds`: active tabs; `focusedWindowKnown:false/currentTabId:null`: focus unknown.

## Read and act
`chrome_execute {sessionId,action,args}`: `untrustedPageContent` receipt. Stale refs refused; unknown never auto-replayed.
`page.read`: `maxChars` characters (default 10000), `limit` refs (default 32), `maxBytes` (default 10000); continue: `textOffset/offset`. Inline reads cap `maxChars/maxBytes` at 12000; file: extension limits. Continue: action/args, app only `cursor`. Top-level `saveTo:"file"`: chat-folder receipt (Ask/Accept edits/Full access); omitted/`"inline"`: inline. Saved ≠ full.
`page.type {ref,text}` appends; `clear:true` replaces. No ref: focused field (`page.type.focused`). `page.check {ref}`: checkbox/radio on; `page.uncheck {ref}`: checkbox off; optional `tabId/frameId`, no `checked` or `value`.
`page.scroll`: `direction` up/down/left/right = 600 px, or `x`/`y` deltas. Click/hover/drag/drop: `ref` or viewport CSS `x`/`y` (including frames); coordinates for ref-less text/images/boxes.
Batch `{sessionId,steps:[{action,args},...]}`: 16 steps/25 s, step mode/receipts; refusal/failure/unknown/`met:false`/`timedOut` stops batch; `nextStep`: first unsent step.

## Access
`history.read`/`bookmarks.read` (Chrome/Edge): ordinary reads, no card in Ask, Accept edits or Full access.
Local addresses (localhost, private networks, link-local, `.local`) use the normal site permission in Safari/extension `browser.localAddresses`. Without `browser.localAddresses`: private networks and this chat's own servers only; otherwise `EXTENSION_UPDATE_REQUIRED`, use `browser_act`. Access modes, tab ownership, password protection and Stop still apply.
`cookies.read` values, `storage.read` values, `clipboard.read` text, `network.body`, `network.headers {unredacted:true}`, `raw.execute`, `page.add_style` run only in Full access, in every browser; ask the user to switch for these reads. Pages/network lists/cookie metadata/storage keys/redacted headers: ordinary reads. Accept edits includes Ask reads/cards for higher-tier writes; vault consent is separate.
`auth.fill {ref,login}`/`auth.basic {url,login}`: vault name, no password args; vault consent. `secret_run {program:"tool",args:[],env:[{"name":"TOKEN","secret":"Token"}]}`: variable/secret ID or name; vault supplies value with consent.

## Files, media
`files.wait_download`: chat download/folder, `finished`/`timedOut`; ambiguous refused. `files.list_downloads`: this chat's session/lease. Upload: chat-relative `paths` or selected `fileIds`.
`clipboard.read/write`: text; `tabs.emulate`: size/colour/touch; `network.intercept` (Chrome/Edge): test responses.
`media.read {tabId}`: audio/video handles/state; play/pause, seek and mute report actual state; `media.transcript`: bounded loaded captions or unavailable. Media: negotiated transport/all modes; scripts: Full access.
`tabs.audio`: muted/audible; `tabs.mute {muted}`: readback; Chrome/Edge need `tabId`; unavailable in Firefox. Built-in Linux/Windows: native; macOS: individual `media.mute` only.
`capture.page`: HTML/Markdown/text in chat folder, with truncation/links. `capture.media`: loaded images/fonts manifest; 5 MiB/file, 16 MiB total, text ≤1M. Same modes as built-in capture.
`page.navigate`: `file:` PDF/HTML/images in chat's first folder, ≤32 MiB, exact owned `tabId`; verifies file/grant. The user sets **Allow access to file URLs** in extension details. Browser renders PDFs; `document_read`: saved PDF when viewer text unavailable.

[Actions](actions.md): profiles, tabs/windows/groups, popups, points/wheel, focus/insert, waits, keys, rich clipboard, site-data clears, retries, Chrome/Edge extras. Firefox: [firefox.md](firefox.md). Safari: [safari.md](safari.md).
