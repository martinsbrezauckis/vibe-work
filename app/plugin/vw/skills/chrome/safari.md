# Your Safari: details

Adds to [actions.md](actions.md) (support table, shared contracts) for Safari profiles (`safari:` profileId, macOS).

Safari's embedded extension follows the app release version. Pairing: the user clicks Pair or Replace soon after opening the pairing page; if pairing fails, ask the user to open the page again and click it right away.

| Topic | Safari behaviour |
|---|---|
| Limits | Frame ids are unusable: read and act in the top document. Popups the page opens are not adopted; open the popup's address in an owned tab or ask the user. New tabs open inactive (`tabs.create` gives `active:false`). Captures, screenshots and recordings work only on the active tab |
| Table export | A long table export can stop early (17 of 41 rows in testing): compare the receipt's row count with the page and report missing rows |
| Media | Autoplay policy refuses `media.play`; pause, seek, mute and `media.read` work |
| Leave page | A "Leave page?" sheet blocks the session and cannot be answered by the agent: ask the user to choose Leave Page in Safari |
| Native input | Native keys, shortcuts and pointer input need the Accessibility permission the user grants Vibe Work; keys are verified with the ABC and U.S. layouts. A brief input marker shows without changing focus; capture removes it; handover to the user, Stop and chat close wait for its removal; it is excluded from page text |
| Forms | Custom numeric datepicker formats from the widget on Safari 18.4+ (`dateFormatChecked`). Native date/time/number/color/range setters, text regions and checkable controls return observed values/states; DOM input is synthetic; visible hit, enabled/inert state and stable geometry are checked first. `page.read {selector}` and `page.select {values}` as in Firefox. `page.scroll` takes a current ref and returns pane movement and boundaries |
| `auth.fill` | Safari `auth.fill` uses vault approval, a secure matching origin and a same-origin POST form; receipts contain no credential values |
| Capture | `capture.screenshot {clip}` and `capture.zoom`: CSS viewport rectangles, exact active tab; clipped/offscreen regions unavailable. Recording: active tab, 512×288 frames, 500 ms each, ≤60 s / 120 frames / 8 MiB; failed or unconfirmed capture is reported; Stop discards unsaved buffers |
| Reads and receipts | Click/commit receipts include observed page state; type/select give changes/full observations with revisions and continuations. Reads support subtree refs, open shadow roots, viewport bounds, counts, continuations; frame omissions and capture limits are in coverage |
| Diagnostics | `console.read`, `page.errors`: bounded page-world messages since attachment, with timestamps and omissions (earlier messages, other frames, object properties unavailable). `network.list`: retained completed Resource Timing metadata; methods, headers, bodies and in-flight requests unavailable |
| Waits and size | Visible text/refs, hidden refs, URL/pattern, DOM readiness with a deadline; network-idle unavailable. `window.resize` affects the chat window; `tabs.emulate` width/height/reset by window resizing, reporting actual viewport and minimum-size mismatches |
| Upload | Visible/hidden file-input ref or one unique top-document file input, ≤16 files / 32 MiB; receipt reads the actual FileList. OS chooser and directory control are unavailable |
| Sharing | Safari's Ask Vibe Work menu, used only by the user, shares website pages, links and images through `person.share.v1`; it requires website access and the original menu document identity (Safari 18.4+) |
| `cookies.change {operation:"clear",tabId,origin}` | Requires Full access and complete website permission coverage. Clears matching host/domain cookies across all paths in the tab's exact nonprivate store, then returns counts after settling; cookies can be shared across subdomains and ports. Partial website grants refuse before deletion; ask the user to allow all websites or clear cookies in Safari. Remaining cookies produce failed; missing read-back produces unknown. Inspect the result before another action; do not automatically repeat a clear |
