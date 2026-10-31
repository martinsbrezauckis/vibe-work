# Chrome Control

Chrome Control connects Chrome to Vibe Work through a paired local helper. It is separate from the app's built-in Browser and App preview.

## Setup

Open Settings > Browser > Your Chrome, choose Set up, review the install offer, then add the Vibe Work extension from the opened listing. While the extension is in Google's review, the setup says so. Open the extension and choose Pair Vibe Work. The connection identity binds that Chrome profile to this app installation. Settings keeps **Use my Chrome** and connection/setup states. Chrome's installation grants are separate from chat permissions.

## Access modes

| Chat mode | Browser actions |
| --- | --- |
| Ask | Read, screenshots, scroll, hover, menus, links, back/forward/reload, open tabs and close the chat's own tabs; console and network reads without bodies. No typing or form submission. |
| Accept edits | Also type, fill, choose, submit forms, answer page dialogs, upload/download, save PDFs, drag/drop, read/add/rename bookmarks, read history and save response bodies. Password and sensitive fields are refused. |
| Full access | Also vault sign-in, cookies/site storage, page scripts, WebMCP tools, bookmark removal, history deletion and unredacted diagnostic headers. Vault consent remains separate. |
| Smart approve | Accept edits. |
| Plan on a page / Explore | Chrome tools are absent. |

Each call and batch step checks the current mode. Above-tier actions return a plain refusal naming the required mode, with no Chrome approval card. There is no first-use card, per-site plugin list, separate Chrome level or tab-closing switch. Old level files are ignored.

## Windows and work state

The vault prompt waits until you answer; saying No answers only that call and the turn goes on, and the agent is
told not to do it another way and to wait for your reply.

Each chat opens an unfocused Chrome window with a named tab group. New tabs go there; your own tabs can also be used. While a chat's agent works in one of your windows, that window is the chat's until its session ends: another chat's agent is refused there. Stop ends work and leaves the window open. Deleting a chat closes its window. Quitting the app leaves windows open and ends sessions. After Stop, the chat's Chrome calls are refused for the rest of that turn. A chat's session ends with its turn; the same chat can continue it in the next turn, in the same window. After a Chrome restart or extension reload, the next call opens a fresh session.

The extension has Stop and connection/work state. Stop remains pending until acknowledged. Connection loss is not a confirmed safe pause. While the agent works in a tab, a frame and a dock reading "Vibe Work is controlling this tab", with Stop, sit on the page; the dock moves to the top edge when it covers what the agent acts on. The frame, pointer and on-page Stop are courtesy indicators that websites can affect; app and extension state are authoritative.

Native Chrome input can activate a window. Screenshots first try without activation; a stalled compositor capture may briefly focus only its chat window and restore previous focus. The receipt records `focusedWindow`. Capturing an external tab never activates its window and can time out if Chrome does not composite it. Uninterrupted typing in a third window is not guaranteed.

## Tools, files and context

`chrome_status {action}` returns an action's schema. `chrome_session` opens, gets, cancels or stops a chat session. `chrome_execute` accepts one action or up to 16 sequential steps within 25 seconds, stopping at the first refusal, failure or unknown outcome. Every page-derived string, including errors and tab metadata, is marked as untrusted page content.

Reads support headings, lists, landmarks, tables, subtrees, depth, interactive elements and optional option lists. CSS-hidden text is excluded. Coordinates resolve nodes and frames before input. Screenshots support regions, scale, maximum dimensions and file output; large full pages are scaled. Console/network reads are filtered and paged from bounded per-tab buffers. Headers redact cookies, authorization and set-cookie by default. Known vault authorization headers stay redacted even at Full access.

Screenshots, PDFs, response bodies and requested GIF recordings land in the chat's `chrome-files` folder. Uploads use chat-relative paths or files selected for that chat in the extension. Handles cannot cross chats. Uploads allow up to 16 files of 32 MiB each; completed downloads copied into the chat folder are limited to 256 MiB.

Ask Vibe Work on selected text adds a chip to that window's chat, or New chat. The composer's Chrome tab picker adds URL, title and current selection. The agent lists and acts in your tabs within the chat's mode; other chats' tabs are not listed to it and are refused. Neither sends a message automatically. WebMCP tools require a supporting Chrome/page and Full access; absence is reported cleanly. Scripts can target a chosen frame. Only the chat's own window can be resized.

## Limits

Ask blocks detected POST form submissions before dispatch, including Space, `requestSubmit` and form-method changes during observation. Arbitrary JavaScript effects cannot be predicted: fetch/XHR POSTs during the observation window are reported as sent, not blocked by the form check. Delayed work outside that window may not be attributed to the click. Receipts describe observed effects, not completed business transactions; unknown outcomes are not replayed.

Vault preparation occurs before the running tool call. Passwords never enter tool arguments, receipts or logs. An iframe field must share the top-page origin and a verifiable same-origin POST form; a formless field without that evidence is refused. HTTP basic sign-in uses the vault, a fixed app label and secure destinations or loopback.

Chrome internal pages and file URLs cannot be driven. The PDF viewer returns its source URL for download. Unsupported frame transforms and custom cross-frame drags are refused. Site permission prompt detection remains incomplete; action deadlines still apply. Native Save as and browser security prompts can require direct interaction. Chrome Control adds no saved-workflow scheduler.

## Network

The agent follows the built-in browser's rule in Chrome: public sites, and this chat's own dev servers on 127.0.0.1, localhost or [::1]. Local-network addresses (private, carrier-grade NAT and unique-local addresses, and names that resolve there) are refused, including a local-network page you opened yourself: the agent does not act on it. Link-local and cloud metadata addresses, other apps' and Vibe Work's own ports on this computer, and names that resolve to this computer are always refused. Each address the agent opens, every frame of a page it acts on, and every request and redirect of its tabs is checked; names are looked up before the request leaves Chrome.

Settings > Developer > Local network pages for the agent in Chrome (off by default) lets the agent use local-network pages in Chrome. The other refusals stay. Turning it off applies at once to the tabs the agent already uses. Vibe Work's built-in browser does not read this switch.

While the agent works on a site with the switch off, that site's background workers (service and shared workers) cannot reach local-network addresses either. With the switch on, they still cannot reach this computer, except the site itself, and they reach named sites over https only. When two chats' dev servers share one host, their pages' background workers cannot reach their servers while both agent tabs are open. A frame inside another site's frame is checked like any other. A local-network page's title is not shown to the agent.

With Local network pages on, the agent's Chrome tabs do not use the browser cache, so pages there load everything from the network again. Your own tabs keep their cache.

While the switch is off, the agent reaches named sites over https only: a plain `http://` address with a name is opened over https, and a site that does not load over https is refused with `HTTPS_REQUIRED`. Addresses written as numbers and `localhost` keep plain http.

A download runs in its own checked tab, so each redirect is checked before Chrome follows it. Each response is also checked by the address Chrome actually connected to, so a name that changes its address after the check is refused before the page reads it. A WebSocket from the agent's tab opens only to a site the rule already allowed in that tab. Limit: a name that changes its address to a local one after the check can still receive a connection attempt from Chrome, but no request.

Remove in Your Chrome removes the helper setup. Chrome and its windows remain. The extension is removed separately in Chrome.
