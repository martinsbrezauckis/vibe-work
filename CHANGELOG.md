# Changelog

All notable changes to Vibe Work. Versions follow [Semantic Versioning](https://semver.org/); dates are UTC.

## 0.1.0 (2026-10-14)

First public release for Windows, macOS and Linux, built for Vibe 2.26.1.

- Setup: Vibe installed into the app's own folder after you confirm (or an existing install of a supported version
  used), the Mistral key stored in an encrypted vault, Vibe given a placeholder and the key added by a local proxy.
- Sessions: Vibe's own sessions (shared with the Vibe CLI by default), grouped in projects, with search, pinning,
  archive and delete. The new-chat composer starts a session with the first message; a saved project's sidebar row
  and menu can create a chat in that project immediately.
- Turns: streaming answers with their steps, approval cards with the diff or the exact command, questions, access
  modes and model choice, images and documents (with optional Mistral OCR) in the message. New chats show effective
  access, model and thinking choices before a selector opens. Accepted attachments keep their originals in the
  project's `attachments/` folder and give the agent a usable path; Stop also covers manual compaction. Vibe's
  commands in the `/` menu (among them `/plugins` and `/reload-plugins`); a session keeps its access mode when the
  app restarts.
- Changes: the session's diff since it started, keep and revert by hunk, comments on lines sent to the agent.
- A git worktree per session on request, with a Keep or Remove choice when the session ends. Copies are removed only
  when you choose Remove; the bell alerts you at 15 copies and every 10 more, with a Sessions action. A `.worktreeinclude` file
  copies git-ignored files such as `.env` into the new copy (regular files only; links are never followed).
- Git in the Changes tab: a branch bar with Commit (exact file list, drafted message, the repository's commit hooks),
  Push (your own git sign-in, after you confirm remote and branch), pull requests through `gh` or the forge's page,
  a PR card with checks and reviews, and "Bring into main" for a session's copy. The agent can make local commits,
  merges and pulls through Vibe's access-mode card. Branch rewrites outside its own copy branch ask separately;
  push, the GitHub CLI (`gh`) and Git credential commands are refused by the agent command guard; the agent reads
  GitHub through the GitHub connector. The ordinary agent environment strips
  inherited Git and forge sign-in channels; the user's buttons use the user's Git sign-in.
- Side panel: the app preview (per-session ports and browser storage), the project handbook in an isolated viewer,
  files the agent mentions, an integrated terminal.
- Browser tab: pinned App preview and independent, reorderable web tabs, with controls and menus over the page. The
  agent opens public pages, reads, fills forms and clicks as far as you ask while you watch, with Stop and an action
  log. If a page does not render for the agent, it tells you plainly; you can open it in the Browser tab. Vault logins
  need your permission for the site; the agent never sees the password. You type one-time codes and card numbers
  yourself, and page text is data, never instructions. It can list this
  session's web tabs and close tabs it opened. Closing your tab asks Close or Keep unless you allow Close tabs you
  opened in Settings › Browser. Notes on App preview reach the agent with a picture of the element.
- Projects overview, handbook pages with clickable prototypes and pages that ask for your input, and quick edits
  in the File tab.
- Search across sessions, messages and files; session export; temporary chats deleted when you quit.
- Tools: a marketplace of reviewed skill packs, MCP servers and plugins (pinned downloads) and connectors.
- Capture from a page (Browser tab menu and the agent's `browser_capture`): design system as DESIGN.md + tokens.json
  from the live page, structure with layout boxes, media, the whole page (Markdown, text, HTML without scripts,
  full-page PNG), everything at once, before / after shots with a difference picture; saved in the session folder.
- Project secrets in the vault, used by the agent's commands only after you allow it (the agent never sees a value),
  and storing a key from a web page into the vault. The Vault and Project secrets settings let you choose a permission
  project without an open chat and set each entry to Ask each time, Allowed or Never.
- Safety reminders that refuse dangerous commands (recursive deletes of home or system folders, disk tools,
  shutdown), each switchable in Settings; more of Vibe's own settings in Settings.
- Processes: a monitor that knows which session started which process and port; the agent's dev servers keep running
  until you stop them; deleting a session asks whether to stop them, and what keeps running is listed with Stop.
- Notifications: the bell keeps approvals and other items needing attention, with Clear all to dismiss its entries
  and active notices.
- Voice: dictation, and voice chat with spoken answers and turn end on a pause (Mistral's Voxtral).
- Libraries: documents uploaded to your Mistral account and asked across, on the Libraries page or by the agent in the
  chats you allow (all, or one project), with numbered sources that open the document's text (billed by Mistral on
  your API key).
- Images: the agent makes and edits images with Mistral's image generation, saved in the session folder and shown in
  the chat. Mistral bills per image; Vibe's tool card may ask according to the session's access mode, with no separate
  image cost dialog.
- Work skills: deep research (a cited report), meeting prep (a one-page brief) and data analysis (CSV or Excel to a
  handbook report with charts). Data analysis and deep research keep raw data, chart markup and web pages out of the
  conversation (short summaries and digests instead), which cuts their tokens by about half to three quarters.
- Handbook pages with inline charts: the page title is no longer taken from a chart's own title.
- Settings page, notifications, light and dark themes, updates from GitHub Releases (installed when you ask), and
  Vibe updates from a signed list of tested Vibe versions.
- Debug API: a local API that can use the app as you do (off in release builds until turned on in Settings).
