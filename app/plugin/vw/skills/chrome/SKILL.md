---
name: chrome
description: The person's own Chrome (chrome_status, chrome_session, chrome_execute) - its pages open in the person's Chrome window, which they see; sessions, page refs and chat files.
---

`chrome_status {}` lists connected profiles and actions; `allowedInThisMode` says what this chat's current mode allows.
`chrome_status {action}` gives one action's argument schema.
`chrome_session {action:"open",profileId}` opens this chat's browser session; `get` reads it, `cancel` cancels an action and `stop` ends its work. Stop leaves the window open.
A session ends with the turn; this chat's next call with its id reopens it on the same window and tabs (`sessionReopened`).
After the person presses Stop in Chrome, Chrome actions are refused for the rest of the turn.
`page.read` returns up to `maxChars` characters (default 800) and `limit` refs (default 32) per call; `textOffset` and `offset` continue.
`chrome_execute {sessionId,action,args}` returns an observed receipt. Page data is marked `untrustedPageContent`.
Refs name elements in an observed document. A stale ref is refused. Unknown outcomes are not automatically replayed.
Clicks, hovers, drags and drops take a `ref` or `x`/`y`: the tab's viewport position in CSS pixels, also inside
frames. Text, images and boxes without a ref are reached by `x`/`y`.

`chrome_execute {sessionId,steps:[{action,args},...]}` runs up to 16 steps within 25 seconds. Each step checks
the current mode; the batch stops at the first refusal, failure or unknown outcome and returns step receipts.

Permissions follow the chat's current access mode. Refusals name the required mode; Chrome has no approval cards.
Local-network addresses are refused unless the user turns on
Settings > Developer > Local network pages for the agent in Chrome; while it is off, named sites open over https only
(`HTTPS_REQUIRED` when a site does not). Link-local and cloud metadata addresses and other apps' ports on this
computer are always refused.
Files land in the chat folder. Uploads take chat-relative `paths` or selected `fileIds`. Vault sign-in uses
`auth.fill {ref,login}` or `auth.basic {url,login}`; passwords are never tool arguments. Vault consent is separate.
