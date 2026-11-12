---
name: chrome
description: The user's own Chrome/Edge/Firefox/Safari window.
---

Extension updates are automatic; app versions are independent. `session_status`: connection/next step, `extensionVersion`, `updateNeeded` (below minimum), reported support. `chromeExtensionStore`: `review`/`available`/`null` = reviewing/listed/unknown.
Settings > Browser > Your Chrome and Settings > Browser > Your Edge connect separately. Removing one browser keeps the other browser connected. `addon_suggest {addon:"chrome"/"edge",reason}`: Connect Chrome: setup; Connect Edge: Your Edge settings, whose initial button is Connect Edge. Only the user installs/pairs/enables. Add a browser tab… dots show connection state; opens ready tab picker or Settings > Browser.
Your Safari (macOS): Settings > Browser > Your Safari switch/pairing; exact `safari:` profileId; no launch/`addon_suggest`. Your Firefox: Vibe Work's own Firefox window (Settings > Browser > Your Firefox > Open window); `firefox:` profileId. Subagents: `browser_read`/`browser_act` only.

`chrome_status {}`: profiles/actions, `allowedInThisMode`, `approvalRequired`; `profileId` filters (`edge`/`Your Edge`). `{action}`: `inputSchemaJson`; `{actions:[...]}`: ≤8 schemas/constraints. `interactiveOnly`, `offset`, `limit`/`maxBytes` ≤500/64000, `includeText`, bounds/revisions: per support.
`chrome_session {action:"open",profileId}` opens; `get` reads; `cancel` cancels action; `stop` ends work, keeps window. Closed paired Chrome/Edge: `profileId:"chrome"`/`"edge"`/exact ID launches, awaits extension ≤10 s; no Ask card. Reconnects: exact ID; failure recovery. Detached browser/tabs outlive Vibe Work.
Chat owns window/tabs. `browserTab`: `profileId,tabId`; open takes `tabId` for a requested existing tab/window. No `tabId`: no selected page; `scope.tabIds:[]` = no tab; `tabs.create {url}`: HTTP/HTTPS. Titles: `session.title`, no title args. Completed turns release control after acknowledgement and retain the session ID; earlier refs are stale. Stop refuses rest-of-turn actions.
CAPTCHA, payment and passkey steps are the user's: an open `ask_user_question` hands this chat's browser to the user; after the answer, read the page and continue.
`tabs.list`: owned/handed windows, blank/internal tabs included. `tabs.close_user` can close any tab there; `tabs.close_owned`: chat-opened tabs. Non-web actions refused. Handed-window tabs stay open until closed. `active`: per window; `selection.activeTabIds`: active tabs; `focusedWindowKnown:false/currentTabId:null`: focus unknown.

`chrome_execute {sessionId,action,args}`: `untrustedPageContent` receipt. Stale refs refused; unknown never auto-replayed.
`page.read`: `maxChars` characters (default 800), `limit` refs (default 8), `maxBytes` (default 2000); continue: `textOffset/offset`. Inline reads cap `maxChars/maxBytes` at 4000; file: extension limits. Continue: action/args, app only `cursor`. Top-level `saveTo:"file"`: chat-folder receipt (Ask/Accept edits/Full access); omitted/`"inline"`: inline. Saved ≠ full.
`page.type {ref,text}` appends; `clear:true` replaces. No ref: focused field (`page.type.focused`). `page.check {ref}`: checkbox/radio on; `page.uncheck {ref}`: checkbox off; optional `tabId/frameId`, no `checked` or `value`.
`page.scroll`: `direction` up/down/left/right = 600 px, or `x`/`y` deltas. Click/hover/drag/drop: `ref` or `x`/`y`: the tab's viewport position in CSS pixels, including frames; coordinates for ref-less text/images/boxes.
Batch `{sessionId,steps:[{action,args},...]}`: 16 steps/25 s, step mode/receipts; refusal/failure/unknown/`met:false`/`timedOut` stops batch; `nextStep`: first unsent step.

`history.read`/`bookmarks.read` (Chrome/Edge): ordinary reads, no card in Ask, Accept edits or Full access.
Local addresses (localhost, private networks, link-local, `.local`) use the normal site permission in Safari/extension `browser.localAddresses`. Without `browser.localAddresses`: private networks and this chat's own servers only; otherwise `EXTENSION_UPDATE_REQUIRED`, use `browser_act`. Access modes, tab ownership, password protection and Stop still apply.
`cookies.read` values, `storage.read` values, `clipboard.read` text, `network.body`, `network.headers {unredacted:true}`, `raw.execute`, `page.add_style` run only in Full access, in every browser; ask the user to switch for these reads. Pages/network lists/cookie metadata/storage keys/redacted headers: ordinary reads. Accept edits includes Ask reads/cards for higher-tier writes; vault consent is separate.
`files.wait_download`: chat download/folder; `finished`/`timedOut` = complete/unfinished; ambiguous refused. `files.list_downloads`: all chats’ Chrome saves. Upload: chat-relative `paths` or selected `fileIds`.
`auth.fill {ref,login}`/`auth.basic {url,login}`: vault name, no password args; vault consent. `secret_run {program:"tool",args:[],env:[{"name":"TOKEN","secret":"Token"}]}`: variable/secret ID or name; vault supplies value with consent.

`clipboard.read/write`: text; `tabs.emulate`: size/colour/touch; `network.intercept` (Chrome/Edge): test responses.
`media.read {tabId}`: main-document audio/video handles/state; `media.play/pause {tabId,mediaId}`, `media.seek {...,timeMs}`, `media.mute {...,muted}`: actual state; `media.transcript`: loaded captions, bounded or unavailable. Media: negotiated transport/all modes; scripts: Full access.
`tabs.audio`: muted/audible; `tabs.mute {muted}`: readback (Chrome/Edge need `tabId`; Firefox: none). Built-in Linux/Windows: native; macOS: individual `media.mute` only.
`capture.page`: passive/packed HTML, Markdown/text in chat folder; reports truncation/links. `capture.media`: loaded images/fonts, saved/unavailable manifest; 5 MiB/file, 16 MiB total; text ≤1M characters. Modes: same as built-in capture.
`page.navigate`: `file:` PDF/HTML/images in chat's first folder, ≤32 MiB, exact owned `tabId`; verifies file/grant. The user sets **Allow access to file URLs** in extension details. Browser renders PDFs; `document_read`: saved PDF when viewer text unavailable.
[More actions](actions.md): per-browser support, open-tab context, pin/group/focus, points/move/wheel, focus/insert, waits, highlight, rich clipboard, page checks, extensions/notifications, retries, Firefox and Safari limits.
