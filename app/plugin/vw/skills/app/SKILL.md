---
name: app
description: Vibe Work, this app around the Vibe agent - its settings, voice, panels, files, attachments and tools.
---

Vibe Work is the desktop app around this Vibe agent. "This app" means Vibe Work. Settings change through the user's controls or a tool that changes them.

- Settings (gear in the rail): General; Appearance; Account; Project secrets; Usage and plan; Models and agent; Browser; Safety; Allowed without asking; Instructions; Voice; Notifications; Privacy and data; Network; Updates; Developer; About.
- Voice: microphone, dictation language, read aloud voice, automatic reading and voice chat. `read_aloud({})` reads your previous reply aloud once (`text` for only part of it). This system's screen reader (named in the Vibe Work note) is separate from these controls.
- Models and agent: defaults for new chats ("New sessions start in") and each model's Compaction point. The access chip below a chat changes only that chat, never the next chat's default.
- Browser: this chat's App preview and web pages. `preview_list` lists this chat's servers and the free leased ports (`leasedFree`) for `process_start`; `vw:preview` and `vw:browser` have the calls.
- Files: the folder tree, file previews and editor. Terminal: shells in the chosen folder. Changes: review edits and commits. Project Pages: plans, reports and comments. `panel_read` reads the open panel and selection; `search` finds text and file names in this chat's folders.
- Tools in the rail: connectors, MCP servers and skills. Libraries: searchable document collections. Vault: keys and logins (`vw:secrets`).
- Paperclip, paste or drop on the chat: attach files, also before the first message. Their Extracted text is in context; `document_read({path, offset})` reads the saved attachment path (nextOffset pages on). A file in this chat's folders is attached and read without a card in Full access or Accept edits; Ask offers a card first. Word, PowerPoint, Excel, CSV and text are read locally; PDF and images use Mistral OCR quota. Dropping a document in Browser does not read it.
- Ctrl/Cmd+K searches actions, settings, chats and files. A waiting answer is on the familiar prompt card; Ctrl/Cmd+Shift+A goes to it, including a subagent's card.
