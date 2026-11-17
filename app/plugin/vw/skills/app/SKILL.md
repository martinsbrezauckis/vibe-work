---
name: app
description: Vibe Work is this app. Settings, voice, panels, files, attachments and tools.
---

This app is Vibe Work.

## Settings
- Gear in the rail: General; Appearance; Accessibility; Account; Project secrets; Usage and plan; Models and agent; Browser; Safety; Allowed without asking; Instructions; Knowledge base; Voice; Notifications; Privacy and data; Network; Updates; Developer; About.
- Browser: Vibe Work browser, Your Chrome, Your Edge, Your Firefox, Your Safari (macOS), All browsers. Tools > Connectors has Chrome card. `addon_suggest({addon, reason})` shows Connect Chrome or Connect Edge cards (`vw:chrome`); `addon` is `chrome` or `edge`. Edge opens Settings > Browser > Your Edge (first button: Connect Edge).
- General: Import from another agent brings setup from supported agents. Close to tray "Keep running in the tray when the window closes" ("menu bar" on macOS). When on with a tray available, closing keeps chats working. A chat notification reopens its chat; tray menu > Quit Vibe Work ends the app.
- Settings search finds familiar terms. Updates offer only tested, supported Vibe versions.
- Network: HTTPS proxy, HTTP proxy, optional Proxy for anything else and Addresses that skip the proxy, and Company certificate file. Its .pem adds trusted certificates. Proxies support HTTP, HTTPS and SOCKS4/5; sign-in stays in the vault. Separate bypass addresses with commas; localhost, 127.0.0.1 and ::1 stay direct. Environment settings override saved values. Settings reports required restarts: Quit Vibe Work and reopen to apply saved changes to Vibe's shell commands; closing to tray is no restart. `process_start` separately inherits the app's startup environment. `session_status.network` reports both proxy routes, exclusions and restart timing without proxy sign-in.
- Voice: microphone, dictation language, read aloud voice, automatic reading and voice chat. `read_aloud({})` reads your previous reply aloud once (`text` for only part of it). System screen reader: separate controls.
- Models and agent: new-chat defaults ("Access mode") and model Compaction point. The access chip below a chat changes only its mode, never defaults.
- Project secrets: "Unlock the vault" offers "With my Windows sign-in", "With my Mac login" or "With my login keyring" by platform, and "With the vault password each time".
- `session_status` reports the chat's model alias, thinking level (null if unknown), access mode, running Vibe version, Vibe Work version, Chrome extension store state and browser add-on states.

## Access modes
Accept edits skips the default approval card for `preview_list`, `preview_console`, `search`, `processes_list`, `port_lease`, `panel_read`, `handbook_answers`, `browser_read`, `browser_capture`, `handbook_check` without `repair` and `preview_screenshot` without `width`. Stricter user rules still apply. Preview resizing/opening and stylesheet repairs still ask. Built-in `browser_act` matches Your Chrome: clicks, typing, form choices, navigation and uploads from the chat's first folder run without a card.

## Chats and window
- Temporary chat starts on Send in an empty folder. Quit deletes Temporary chats; startup retries failed deletions. Busy folder removal retries briefly on chat deletion, then at Quit or startup once no temporary chats remain. Changes outside that folder remain.
- Needs you: unanswered chats. Windows taskbar/macOS Dock show a count; tray/menu bar a dot ("Vibe Work: 1 chat needs your answer"); Linux trays show "Needs your answer". Viewing clears the count until the next prompt. Windows Notification Center opens the chat after the banner disappears.
- Ctrl/Cmd+K searches actions, settings, chats and files. Waiting answers appear on prompt cards. Ctrl+R searches past prompts in the composer; choosing one fills the draft without sending. Ctrl/Cmd+Shift+A opens a waiting answer, including a subagent's card.
- Send feedback: title bar, Settings > About, tray or `/feedback`. Usage and plan shows monthly use when reported; Context % opens Context breakdown.
- A rendering error shows "The window could not be displayed" with Try again and Reload window (reloads the whole window). A startup error shows "Vibe Work could not start", its reason and "Quit and try opening Vibe Work again."

## Panels
- Browser: App preview/web pages. Files offers Open and Show in folder for downloads even in Temporary chats. `preview_list` lists chat servers and free leased ports (`leasedFree`) for `process_start`; see `vw:preview` and `vw:browser`.
- Files: tree, previews, editor. Terminal: shells in folder. Changes: edits and commits. Project Pages: plans, reports, comments. `panel_read` reads open panel/selection; `search` finds text and file names in this chat's folders.
- Tools: connectors, MCP servers, skills; plugin switches sync with Vibe. Libraries: document search. Vault: keys and logins (`vw:secrets`). Processes > Left running lists agent processes surviving chat deletion, with Stop (`vw:processes`).
- Folder pins in `.vibe/skills.toml` install from Mistral's skill registry on chat start, without asking; registry skills are on by default. Unfetchable pins (unknown id, no connection) are silently skipped; the chat starts without them. Tools > Skills lists On this computer and Mistral's catalog, which can be empty.

## Files
- Paperclip, paste or drop attaches files, even before the first message. Extracted text enters context; `document_read({path, offset})` reads an attachment or a file in the chat's folders without copying (nextOffset pages on): Word, PowerPoint, Excel, CSV and text read locally without a card in any mode. PDF and images use Mistral OCR quota and ask for approval in Ask. Dropping a document in Browser does not read it.
- `video_frames({path, selection, output})` (when listed in your tools) reads video up to 512 MiB and at most two hours. Selection: `uniform`, `interval`, `timestamps` or `scene_changes`; output: `frame`, `contact_sheet` or `metadata`. Limit: 24 frames across 10 minutes; `frame` requires one. Metadata sends no image; no audio. Planned frame/image counts appear before extraction. Decoding depends on this computer; refusals distinguish formats, worker failures and limits. If unavailable, ask for still images.

## Other tools
- `connectors_list` lists Mistral connectors; `connector_suggest` offers one for review.
- `feedback_draft({kind, title, description, remarks?})` prepares an editable report the user sends; `kind`: `bug` or `feature`.
- `use_headless_browser` enables a chat's installed headless-browser developer plugin; tools arrive after this reply.
- `propose_protected_change` proposes protected agent settings changes; `protected_change_status` reports the user's decision.
