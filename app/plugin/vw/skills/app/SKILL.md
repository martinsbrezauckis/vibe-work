---
name: app
description: Read this for questions about Vibe Work, this app, its settings, voice, panels, files or tools.
---

Vibe Work is the desktop app around this Vibe agent. "This app" means Vibe Work. Describe the controls the user can open; claim a setting changed only after a tool changes it.

- Settings (gear in the rail): General; Appearance; Account; Project secrets; Usage and plan; Models and agent; Browser; Safety; Allowed without asking; Instructions; Voice; Notifications; Privacy and data; Network; Updates; Developer; About.
- Voice: microphone, dictation language, read aloud voice, automatic reading and voice chat. This system's screen reader (named in the Vibe Work note) is separate from these controls.
- Models and agent: defaults for new chats, including "New sessions start in". The access chip below a chat changes only that chat, never the next chat's default.
- Browser: this chat's App preview and web pages. Call `preview_list` first; start a server with `process_start` on a `leasedFree` port. Read `vw:preview` or `vw:browser` for exact calls.
- Files: the folder tree, file previews and editor. Terminal: shells in the chosen folder. Changes: review edits and commits. Project Pages: plans, reports and comments. `panel_read` reads the open panel and selection.
- Tools in the rail: connectors, MCP servers and skills. Libraries: searchable document collections. Vault: keys and logins; read `vw:secrets`.
- Paperclip, paste or drop on the chat: attach files, also before the first message. Use the Extracted text in context, or `document_read({path, offset})` with the saved attachment path (nextOffset pages on); a file in this chat's folders is attached and read without a card in Full access or Accept edits; Ask offers a card first. Word, PowerPoint, Excel, CSV and text are read locally; PDF and images use Mistral OCR quota. Say when text is unreadable; never summarize from metadata. Dropping a document in Browser does not read it.
- Ctrl/Cmd+K searches actions, settings, chats and files. A waiting answer is on the familiar prompt card; Ctrl/Cmd+Shift+A goes to it, including a subagent's card.
