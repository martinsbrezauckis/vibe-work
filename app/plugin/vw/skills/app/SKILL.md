---
name: app
description: Vibe Work, this app around the Vibe agent - its settings, voice, panels, files, attachments and tools.
---

Vibe Work is the desktop app around this Vibe agent. "This app" means Vibe Work. Settings change through the user's controls or a tool that changes them.

- Settings (gear in the rail): General; Appearance; Accessibility; Account; Project secrets; Usage and plan; Models and agent; Browser; Safety; Allowed without asking; Instructions; Voice; Notifications; Privacy and data; Network; Updates; Developer; About.
- Settings > Browser groups Vibe Work browser, Your Chrome, Your Edge and All browsers. Your Chrome and Your Edge setup is here; Chrome's connection card also appears above the connectors in Tools > Connectors.
- `session_status` reports this chat's access mode, running Vibe version, Chrome extension store state and browser add-on states. `addon_suggest` takes `addon` (`chrome` or `edge`) and a `reason` to show the person's Connect Chrome or Connect Edge card (`vw:chrome`).
- Temporary chat can start on Send without choosing a folder, in an empty folder of its own. Quit deletes Temporary chats; failed deletions are retried at startup. Vibe Work tries to remove the folder when its chat is deleted. If it cannot, cleanup retries at Quit or startup once no temporary chats remain. Changes outside that folder stay changed.
- Settings > General's Close to tray option is labelled "Keep running in the tray when the window closes" ("menu bar" on macOS). With it on and a tray available, closing the window keeps chats working. Clicking a chat notification reopens that chat; Quit Vibe Work in the tray menu ends the app.
- Settings > Network has HTTPS proxy, HTTP proxy, Proxy for anything else (optional), Addresses that skip the proxy (optional), and Company certificate file (.pem, added to trusted certificates). Proxy addresses support HTTP, HTTPS and SOCKS4/5; proxy sign-in stays in the vault. Addresses that skip the proxy are comma-separated; localhost, 127.0.0.1 and ::1 stay direct. The computer's environment settings override saved values. Settings reports when a restart is needed: saved changes apply fully to the agent's own commands after using Quit Vibe Work and opening the app again; closing to the tray is not a restart.
- Voice: microphone, dictation language, read aloud voice, automatic reading and voice chat. `read_aloud({})` reads your previous reply aloud once (`text` for only part of it). This system's screen reader (named in the Vibe Work note) is separate from these controls.
- Models and agent: defaults for new chats ("Access mode") and each model's Compaction point. The access chip below a chat changes only that chat, never the next chat's default.
- In Accept edits, `preview_list`, `preview_console`, `search` and `preview_screenshot` without a `width` run without the default approval card; stricter user rules still apply. A screenshot that sets a width, `preview_open`, `browser_read`, `panel_read`, browser clicks and typing still ask.
- Browser: this chat's App preview and web pages. `preview_list` lists this chat's servers and the free leased ports (`leasedFree`) for `process_start`; `vw:preview` and `vw:browser` have the calls.
- Files: the folder tree, file previews and editor. Terminal: shells in the chosen folder. Changes: review edits and commits. Project Pages: plans, reports and comments. `panel_read` reads the open panel and selection; `search` finds text and file names in this chat's folders.
- Tools in the rail: connectors, MCP servers and skills. Libraries: searchable document collections. Vault: keys and logins (`vw:secrets`).
- Processes > Left running lists agent processes that survived deletion of their chat, with Stop (`vw:processes`).
- Paperclip, paste or drop on the chat: attach files, also before the first message. Their Extracted text is in context; `document_read({path, offset})` reads an attachment, or a file in this chat's folders in place without a copy (nextOffset pages on): without a card in Full access or Accept edits; Ask offers a card first. Word, PowerPoint, Excel, CSV and text are read locally; PDF and images use Mistral OCR quota. Dropping a document in Browser does not read it.
- Ctrl/Cmd+K searches actions, settings, chats and files. A waiting answer is on the familiar prompt card; Ctrl/Cmd+Shift+A goes to it, including a subagent's card.
