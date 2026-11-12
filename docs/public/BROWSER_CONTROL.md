# Chrome Control

Chrome Control connects Chrome to Vibe Work through a paired local helper. It is separate from the app's built-in Browser and App preview.

## Setup

Choose **Connect Chrome** in Tools or Settings > Browser > Your Chrome. On Windows and macOS, Chrome asks you to enable the Vibe Work extension. If Chrome was already running on macOS, restart it when the card says **Restart Chrome**. On Linux, the button opens the extension's Store page in Google Chrome; choose **Add to Chrome** and accept Chrome's prompt. Snap and Flatpak browsers and other Chromium browsers are not supported.

Vibe Work opens the extension’s pairing page. When the extension asks to pair, a Vibe Work dialog asks **Connect Chrome to Vibe Work?**; choose **Connect**. With an extension version that supports it, pairing finishes by itself: the page checks that it is this Vibe Work in this Chrome profile, pairs, and closes once Chrome is connected. Otherwise, or when anything does not match, the page asks as before: compare its connection identity with the card, then choose **Pair Vibe Work**. A Chrome already paired with another Vibe Work always asks before it is replaced. The card follows the connection and shows **Connected** when Chrome is ready; after pairing, a closed Chrome shows **Connected when Chrome is open**. New chats can then use Chrome within their access mode. Settings keeps **Use my Chrome** and Remove. Chrome's installation grants are separate from chat permissions.

If Chrome turned the extension off, for example after an update that asked for new permissions, the card shows **Turn on in Chrome**; turn it on on Chrome's Extensions page. When this Vibe Work needs a newer extension, the card shows **Update the extension**. While the extension is in Google's review, the card shows **Extension in review**. If Chrome has previously removed the extension or cannot fetch it, choose **Connect Chrome** again to open its Store page. Private listings require an eligible signed-in Google account.

On macOS, you are asked once for permission when the helper reads a home folder or Chrome profile on an external drive.

Remove on **Your Chrome** disconnects only Chrome; Remove on **Your Edge** disconnects only Edge. The other browser keeps its connection.

## Access modes

| Chat mode | Browser actions |
| --- | --- |
| Ask | Read, screenshots, scroll, hover, menus, links, back/forward/reload, open and close tabs, select options, check and uncheck, and download files; console and network reads without bodies. Reading history and bookmarks runs without a card. Typing, uploads, drag/drop, accepting page dialogs and explicit sends need permission. |
| Accept edits | Also type, fill, submit forms, answer page dialogs, upload, save PDFs, drag/drop, read/add/rename bookmarks and read history. Typing into password and sensitive fields is refused. |
| Full access | Also cookie values, site-storage values, clipboard text, captured network response bodies and unredacted diagnostic headers, vault sign-in, site-data changes, page scripts, WebMCP tools, bookmark removal and history deletion. Vault consent remains separate. |
| Smart approve | Accept edits. |
| Plan on a page / Explore | Chrome tools are absent. |

Cookie values, local/session storage values, clipboard text, captured network response bodies and unredacted request/response headers require Full access in every browser, including the built-in browser. Ask and Accept edits refuse these reads with a reason and instructions for switching modes. Captured response bodies can contain credentials that were never displayed on the page. Rendered page content, cookie names and storage keys remain ordinary reads. Access modes are cumulative: Accept edits retains Ask's approval cards for actions it does not run directly, including HTTP sign-in with a saved vault login.

Each call and batch step checks the current mode. Opening a paired, closed Chrome or Edge runs without a card. Secret reads below Full access return a refusal with a reason and instructions; other actions above the mode's delegated tier use a browser approval card. There is no first-use card, per-site plugin list, separate Chrome level or tab-closing switch. Old level files are ignored.

Scroll results state when the page or selected scroll pane reaches its edge, including when another scroll cannot move it.

## Windows and work state

The vault prompt waits until you answer; saying No answers only that call and the turn goes on, and the agent receives:
`Not run: the user declined this. Explain why it is needed for the user's delegated task.`

Each chat opens an unfocused Chrome window. New tabs go there and are selected by default; your own tabs can also be used. Chat windows stay ungrouped, so closing one leaves no automatically saved tab group. While a chat's agent works in one of your windows, that window is the chat's until its session ends: another chat's agent is refused there. Stop ends work and leaves the window open. Deleting a chat closes its window. Quitting the app leaves windows open and ends sessions. After Stop, the chat's Chrome calls are refused for the rest of that turn. When a turn finishes, its frame and action marker disappear after the browser confirms release. The session stays available for follow-up in the same window, with the same identity; browser work reacquires control before acting. Thirty minutes without browser work ends the retained session. Stop, closing the chat, quitting the app or closing its retained target also ends it; Stop and idle expiry leave the person's tabs open. After a Chrome restart or extension reload, the next call opens a fresh session. Reopening after a lost connection or an extension reload keeps the chat's selected tab when the same page is still open; previous page references and unfinished actions are not reused.

The extension has Stop and connection/work state. Stop remains pending until acknowledged. Connection loss is not a confirmed safe pause. While the agent works in a tab, a frame and a dock reading "Vibe Work is controlling this tab", with Stop, sit on the page; the dock moves to the top edge when it covers what the agent acts on. The frame, pointer and on-page Stop are courtesy indicators that websites can affect; app and extension state are authoritative.

Native Chrome input can activate a window. Screenshots first try without activation; a stalled compositor capture may briefly focus only its chat window and restore previous focus. The receipt records `focusedWindow`. Capturing an external tab never activates its window and can time out if Chrome does not composite it. Uninterrupted typing in a third window is not guaranteed.

When the agent asks you a question, it releases this chat's browser so you can complete a sign-in or challenge. Browser reads and actions wait for your answer; other chats can continue. Skip leaves browser work waiting for a new message. Stop cancels it without closing the page.

## Tools, files and context

`chrome_status {action}` returns an action's schema. `chrome_session` opens, gets, cancels or stops a chat session. `chrome_execute` accepts one action or up to 16 sequential steps within 25 seconds, stopping at the first refusal, failure, unmet condition or unknown outcome. A wait that times out prevents later steps from running. Every page-derived string, including errors and tab metadata, is marked as untrusted page content.

Reads support headings, lists, landmarks, tables, subtrees, depth, interactive elements and optional option lists. CSS-hidden text is excluded. Coordinates resolve nodes and frames before input. Screenshots support regions, scale, maximum dimensions and file output; large full pages are scaled. Console/network reads are filtered and paged from bounded per-tab buffers. Headers redact cookies, authorization and set-cookie by default. Known vault authorization headers stay redacted even at Full access.

Screenshots, PDFs, response bodies and requested GIF recordings land in the chat's `chrome-files` folder. Uploads use chat-relative paths or files selected for that chat in the extension. Handles cannot cross chats. Uploads allow up to 16 files of 32 MiB each; completed downloads copied into the chat folder are limited to 256 MiB.

Ask Vibe Work on selected text adds a chip to that window's chat, or New chat. With extension 0.1.7, the same menu also shares a page, a link or an image. Pages add their URL, title and current selection and hand the window to the chat. Links and images add their addresses and source-page details without opening or downloading them. The composer's Chrome tab picker adds URL, title and current selection. The agent lists and acts in your tabs within the chat's mode; other chats' tabs are not listed to it and are refused. Neither sends a message automatically. WebMCP tools require a supporting Chrome/page; invoking them needs browser approval below Full access; absence is reported cleanly. Scripts can target a chosen frame. Only the chat's own window can be resized.

With Full access, the agent can clear local or session storage for an exact site in Your Chrome or Your Edge and check how many entries remain. This does not clear other site stores or all cookies. The built-in browser also requires Full access for clearing site data.

## Limits

Ask blocks detected POST form submissions before dispatch, including Space, `requestSubmit` and form-method changes during observation. Arbitrary JavaScript effects cannot be predicted: fetch/XHR POSTs during the observation window are reported as sent, not blocked by the form check. Delayed work outside that window may not be attributed to the click. Receipts describe observed effects, not completed business transactions; unknown outcomes are not replayed.

Vault preparation occurs before the running tool call. Passwords never enter tool arguments, receipts or logs. An iframe field must share the top-page origin and a verifiable same-origin POST form; a formless field without that evidence is refused. HTTP basic sign-in uses the vault, a fixed app label and secure destinations or loopback.

Chrome internal pages and file URLs cannot be driven. The PDF viewer returns its source URL for download. Unsupported frame transforms and custom cross-frame drags are refused. Site permission prompt detection remains incomplete; action deadlines still apply. Native Save as and browser security prompts can require direct interaction. Chrome Control adds no saved-workflow scheduler.

## Network

With an extension version that supports it, Your Chrome and Your Edge allow local addresses with the normal per-site permission: localhost, local development servers, private networks, link-local addresses, `.local` names and router pages. Navigation, redirects, frames, downloads, page requests and background workers use the browser's normal network behavior. Browser security and local-network permission prompts still apply. Cache and plain HTTP work normally.

Site permissions, chat access modes, tab ownership, password protection and Stop still apply. Local addresses need no Developer switch. The built-in browser also adds no local-address restriction; its engine decides what it can reach.

Your Chrome and Your Edge require extension 0.1.7 or later.

Media downloaded by Vibe Work outside a browser retains its separate address checks, including private-address refusals and the existing exceptions for the current page's own local resources.

Remove in Your Chrome removes the helper setup and any extension-install registration it created. Chrome and its windows remain. Chrome may remove an externally installed extension when it next checks the registration; you can also remove the extension in Chrome.

## Your Safari

On macOS, Settings > Browser > Your Safari connects Safari through the Safari extension inside Vibe Work. Turn on **Use my Safari** and choose **Open pairing page**; turn on the Your Safari extension in Safari's settings if Safari asks, then pair from Safari's toolbar (Your Safari, then Pair) after comparing its identity with Vibe Work. Your Safari has its own switch, separate from Your Chrome and Your Edge.

Access modes, the chat's own window, handed windows, tab closing and Stop work as in Your Chrome. Safari supports native date, time, number, color and range fields, text regions and checkboxes with the resulting value or state read back. Safari supports fewer actions overall; the agent sees which in its status. Vibe Work does not open Safari for the agent: keep Safari open while it works. Safari's own website permissions decide which public and local pages open. **Remove…** removes the Safari connection from Vibe Work.

The built-in browser can focus or blur an observed control and insert text at its current selection without clearing the surrounding text. Insertions send input events without keyboard events, and the final receipt checks the resulting value.

Browser waits can distinguish an element becoming hidden from leaving the document, and can wait for DOM readiness, a particular address, or a bounded delay. A timeout reports that the condition was not met.

The built-in browser can scroll a particular pane on either axis and read its current position and edges without moving the surrounding page.

Open-tab context lists titles and addresses and reads a selected page without activating it or taking over its window. Other chats’ tabs are excluded. Firefox exposes only its connected agent window; separate personal profiles are outside that connection.

Chrome and Edge can pin or unpin the agent’s own tabs and report their actual pin state. Personal tabs outside the chat’s owned window keep their pin state.

Chrome and Edge can focus the chat’s window while keeping its current active tab.

Chrome and Edge can group the agent’s tabs, name their groups, and remove a tab from its group. A window containing personal or unobserved tabs is refused for group changes. Group removal is confirmed from a fresh tab inventory. Supplying or changing a group title asks for approval in Ask; Accept edits and Full access include these writes.

The built-in browser blocks an identical agent input when an earlier native input was sent but its result could not be verified. Reading the page again does not erase that unresolved outcome.

The built-in browser can move the pointer to an observed point or control without clicking, including canvas and SVG targets. Changed or replaced targets require a fresh observation.

An observed point can be resolved to its current target without clicking or moving the pointer. Covered controls and changed frame geometry are refused.

`page.resolve_point` returns a short-lived pointId for a visible node in an owned Chrome, Edge or connected Firefox tab. `page.move` and `page.wheel` use that pointId, recheck the same node and geometry, and report the observed native event and actual scroll offsets. Wheel accepts deltaX/deltaY in pixels. Covered or changed points refuse before input; points inside child frames use existing frame element actions.

`page.focus`, `page.blur` and `page.insert_text` use pointId in Chrome/Edge and an observed ref in Firefox. Insertion replaces the current selection, preserves surrounding text and editable markup, sends beforeinput/input without key events, and reports actual focus/value. Sensitive, replaced or canceled fields return a refusal or met:false with a reason.

`page.wait_for` in Chrome/Edge and Firefox polls one CSS selector for attached, detached, visible or hidden. A hidden element remains attached. Conditions are read-only and bounded by timeoutMs (up to 20000); timeout returns met:false, and ambiguous or invalid selectors return a reason.

`clipboard.read_rich` and `clipboard.write_rich` in Chrome/Edge require Full access. They take mime text/html or image/png; writes take text for HTML or base64 for PNG. Writes return actual clipboard content read back, including whether its bytes match the input. PNG reads return an image. Content is bounded to 256 KiB, HTML input to 60000 bytes; temporary transfer files remain private to the operation and are removed at completion. The chat mode itself must be Full access because writes also read clipboard content back; a write approval in a lower mode cannot authorize that readback.

`clipboard.read_rich {mime}` and `clipboard.write_rich {mime, text? | base64?}` use the native clipboard in Full access. MIME is text/html or image/png; HTML input uses text (up to 60000 bytes), PNG uses base64 (up to 256 KiB decoded). Writes read back actual content, PNG reads return an image, and active vault copies are withheld. HTML is supported on Windows, macOS and Linux X11; the current Wayland backend refuses HTML with a reason and supports PNG/plain text. The chat mode itself must be Full access because writes also read clipboard content back; a write approval in a lower mode cannot authorize that readback.

The built-in browser, Chrome, Edge and connected Firefox can report the chat’s window size and focus without bringing it forward.

On Windows and Linux the built-in browser can send wheel input at an observed element or point and report the actual scroll position. A covered or replaced target is refused. On macOS, ordinary scrolling remains available; native wheel input reports that its delivery route is unavailable.

`page.scroll_into_view {selector,tabId?}` brings one unique main-document element into view in Chrome, Edge or connected Firefox and returns actual bounds and scroll-container offsets. Hidden or ambiguous selectors do not scroll. Safari’s current adapter refuses this route; ordinary scrolling remains available.

In Safari, right-click a webpage, link or image and choose **Ask Vibe Work** to add it to the app. Sharing requires website access and Safari 18.4 or later, and checks the document where the menu opened. A page already open when the extension is updated may need a reload before sharing.
