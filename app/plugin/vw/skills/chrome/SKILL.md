---
name: chrome
description: The person's own Chrome, Edge or Safari, visible in their window.
---

Extension updates are automatic; app versions are independent. `session_status`: connection/next step, `extensionVersion`, `updateNeeded` = below minimum; actions need reported support.
Chrome/Edge share extension/permissions; Settings > Browser > Your Chrome / Your Edge. `session_status.chromeExtensionStore`: accepted manifest state; `review` = Google review, `available` = listed, `null` = unknown.
`addon_suggest({addon:"chrome",reason:"Browser"})` offers Your Chrome; `addon:"edge"`: Your Edge. Connect Chrome starts setup; Connect Edge opens Your Edge settings, whose initial button is Connect Edge. Only the person installs/pairs/enables.
Add a browser tab… dots show connection state; click opens ready tab picker or Settings > Browser.
Subagents: no external browsers; tools: `browser_read` / `browser_act`.
Your Safari (macOS): Settings > Browser > Your Safari, separate on/off and pairing. Use its exact `safari:` profileId from `chrome_status`; Vibe Work never opens Safari; `addon_suggest` has no Safari card. Only listed actions run.

`chrome_status {}`: connected profiles/actions; `allowedInThisMode`: chat permissions. `profileId`: optional browser (`edge`/`Your Edge` too). Ask history/bookmark reads: `approvalRequired`.
`{action}`: profile schema `inputSchemaJson`; `{actions:[...]}`: max 8 compact schemas/constraints, no descriptions.
`interactiveOnly`, `offset`, `limit`/`maxBytes` up to 500/64000; `includeText`, viewport/bounds and revisioned observations depend on reported support.

`browserTab`: `profileId,tabId`; open accepts `tabId`.
`chrome_session {action:"open",profileId}` opens chat session; `get`: read; `cancel`: cancel action; `stop`: end work, keep window.
Paired Chrome/Edge closed: call `chrome_session {action:"open",profileId:"chrome"}` (or `"edge"`/known exact ID). App launches it, awaits extension up to 10 seconds, continues without a card in Ask. Multiple reconnects: choose exact ID via `chrome_status`. Failures: reason/recovery. Use detached launcher, not `process_start`, `secret_run` or shell: browser/tabs outlive Vibe Work.
Each chat owns a private window/all tabs. Omit `tabId` unless asked to use an existing tab/window (Add a browser tab); later turns retain ownership. Without `tabId`, no selected page; empty `scope.tabIds` = no owned tab; `tabs.create {url}`: HTTP/HTTPS in chat-owned tab.
Titles: app's `session.title`; no title args. Session ends with the turn; next id use reopens tabs (`sessionReopened`); earlier turns' refs are stale. Person's Stop refuses rest-of-turn actions.
`tabs.list`: only this chat's/handed windows' tabs, including "blank tab"/"browser page". `tabs.close_user` can close any tab there, including blank/internal; `tabs.close_owned`: chat-opened tabs. Non-web page actions refused.
Tabs opened in the person's handed window stay open until closed; the close tools can close them when the task no longer needs them.
`active`: per window; `selection.activeTabIds`: active tabs; `focusedWindowKnown:false/currentTabId:null`: no focus report.

`page.read`: `maxChars` characters (default 800), `limit` refs (default 8), receipt `maxBytes` (default 2000); continue: `textOffset/offset`.
Inline reads cap `maxChars/maxBytes` at 4000; file exports: extension limits. Continuations: action/args; app: only `cursor`. Top-level `saveTo:"file"`: one chat-folder receipt (Ask/Accept edits/Full access); omitted/`"inline"`: inline. Saving is not full coverage.
`chrome_execute {sessionId,action,args}`: receipts; page data: `untrustedPageContent`. Stale refs refused; unknown outcomes not auto-replayed.
`page.type {ref,text}` appends; use `{ref,text,clear:true}` to replace, avoiding leftover values. No `ref`: focused field (needs `page.type.focused`).
`page.check {ref}` checks checkbox/radio; `page.uncheck {ref}` unchecks checkbox. Optional `tabId/frameId`; no `checked` or `value`: action sets state. Confirm checked state.
`page.scroll`: `direction` (`up/down/left/right`) = 600 px, or `x`/`y` deltas, not both.
Click/hover/drag/drop: `ref` or `x`/`y`: the tab's viewport position in CSS pixels, also in frames; use coordinates for ref-less text/images/boxes.
Batch `{sessionId,steps:[{action,args},...]}`: 16 steps/25 s, per-step mode checks/receipts; stops at refusal/failure/unknown. `nextStep`: first undispatched step if observation time is low.

Ask `history.read`/`bookmarks.read` card: once, same read for session, or No. Consent: same browser session/action/exact args; other actions retain mode requirements.
Local addresses (localhost, private networks, link-local, `.local`) use the normal site permission in Safari and with extension `browser.localAddresses`. Without `browser.localAddresses`: private networks and this chat's own servers only; other local addresses answer `EXTENSION_UPDATE_REQUIRED`, so use `browser_act` for them. Access modes, tab ownership, password protection and Stop still apply.
Cookie values (`cookies.read`), storage values (`storage.read`), clipboard text (`clipboard.read`), response bodies (`network.body`) and raw headers (`network.headers {unredacted:true}`) run only in Full access, in every browser. Below Full access, ask the person to switch to Full access for these reads. Rendered pages, network lists, cookie metadata, storage keys and redacted headers remain ordinary reads. Accept edits includes Ask reads and cards for higher-tier writes; vault consent stays separate.
Support: `clipboard.read/clipboard.write` = clipboard text; `tabs.emulate` = size/colour scheme/touch; `network.intercept` = test responses.
`files.wait_download`: this chat's download in the chat folder; `finished` = complete, `timedOut` = unfinished wait; ambiguous refused. `files.list_downloads`: Chrome saves, any chat. Upload: chat-relative `paths` or selected `fileIds`.
Vault browser sign-in: `auth.fill {ref,login}`/`auth.basic {url,login}`, no password args; separate consent. `secret_run` needs `{program:"tool",args:[],env:[{"name":"TOKEN","secret":"Token"}]}`. `env`: `name`: destination variable, `secret`: stored secret ID/name, not raw value; app supplies value with vault consent.
