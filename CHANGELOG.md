# Changelog

All notable changes to Vibe Work. Versions follow [Semantic Versioning](https://semver.org/); dates are UTC.

## 0.1.3 (2026-10-30)

Built for Vibe 3.0.3 on Python 3.14.8. This release adds @-mentions, edit previews with line numbers and connectors that need no sign-in.

- @-mentions: type @ in the message box to mention a file or another session in the same folder. Files are found by
  fuzzy name match; a session adds a chip with its title, first message and last answer.
- Edit previews: edit and write cards show the change with old and new line numbers and its +/− counts, as the file
  was when the card opened. A shortened preview says what it left out; Show the whole change shows all of it.
  Previews cover files inside the chat's folders.
- Large 4 offers Low, Medium and High thinking, and each runs as picked.
- When your default model is no longer offered, Vibe Work asks at launch: choose one of the models Vibe offers, or
  Later.
- Settings > Models and agent > Compaction point: choose for each model when a chat's history is compacted, or keep
  Default.
- Tools > Connectors: a connector that needs no sign-in shows Connect. One click connects it, and the next message
  can use its tools.
- Tools > Skills shows when each skill you added last changed. The built-in help skill is named Vibe Work help.
- Vibe's configuration problems appear in the bell, one line each, with the file or folder first where there is one.
  Each stays until you dismiss it; a dismissed one does not come back.
- GLM 5.3 Flash shows its 1M-token context.
- App preview: pages you signed in to before the first message stay signed in after you send it.
- Saying No in one of Vibe Work’s own prompts (Git guard, processes, vault, documents, protected folders, cards for
  Vibe Work’s tools) answers only that call and the turn goes on; the agent is told not to do it another way and to
  wait for your reply. A No on a card for Vibe’s own tools (commands, file edits) still stops the turn and the
  subagents it started.
- macOS: Quit always finishes, also right after a reply.
- Chat titles no longer show a vault value once the vault has been unlocked in this run: the matching part reads
  [secret withheld] in the list, the header, the bell and the Debug API.
- Point Vibe at this: an element picked in the App preview is marked as the page's words, and the chip names the
  address the preview loaded, or none when the page changed during the pick.
- Attached documents: the text read from them counts toward the 200 MB limit, and a PowerPoint deck that lists the
  same slide twice is refused.
- In a chat started in a subfolder, files are named from that subfolder and links in Vibe's replies open the file
  there first. Files elsewhere in the project show their full path.
- A chat waiting on one of Vibe Work's own prompts (Git guard, vault, document card) shows Needs input and is listed
  under Needs you.
- Secret-scan entries leave the bell once their finding is gone, for example after the chat is deleted.
- Continues in CLI appears only on the chat that `vibe --continue` in a terminal resumes.
- Action cards without a summary name the action, such as Read an open editor, and show no empty input.
- Settings > Notifications and the New chat copy section show no explaining lines.
- Layout at 900 px: the chat title stays readable with the chat list open, header tabs show only their icons when
  the chat area is narrow, and vault names show in full.

## 0.1.2 (2026-10-29)

Built for Vibe 3.0.3 on Python 3.14.8. This release adds files and history, editors side by side and clearer permissions.

- Files tab: browse your project's folders, search file names and text, and open a result at its line. History shows
  who saved each version, when, what changed and what has not been published. The Git tab's actions are here too.
- Work before the first message: choose a folder on New chat and use Files, the editors, Terminal, Browser and
  Changes. Send your first message when you want Vibe to join you; what you opened stays open.
- Editors: each file has its own tab; Open to the side puts two files beside each other. Compare with saved version
  shows the saved text beside your edits. Settings > Appearance > Open files in also offers One File tab.
- Edit together: Vibe can read this chat's open files and selection. Its changes appear with marked lines; edits on
  different lines merge with yours, and changes to the same lines offer Keep yours, Take Vibe's or Compare side by side.
  Settings > General > Save open files before Vibe works is on by default.
- File previews: HTML and Markdown open as a preview, with Edit and Preview to the side; pictures and PDFs open as
  previews. Choose how HTML and Markdown open in Settings > Appearance.
- Saying No on any approval, including a subagent’s, stops the whole turn. Approvals wait until you answer.
- Permission prompts hold the current call until you answer; Allow runs it once in the same turn.
- Permission cards: commands, edits, questions and changes to Vibe's settings use the same card.
  Allow once covers one action; Allow for this session covers the actions the card describes. The session header
  lists those permissions with Revoke. Review lasting choices in Settings > Allowed without asking.
- Ctrl/Cmd+Shift+A goes to the open chat's waiting prompt, or to the oldest one in another chat when the open chat
  has none; answering returns focus to the message box. A prompt that opens in its own window comes to the front
  when you open its chat, click the chat's waiting status, choose Open session in the bell or press the shortcut;
  with several waiting in one chat, each press shows the next one.
- Prompt windows for a chat's Git, vault and saved-login requests keep the rest of the app usable: you can work in
  other chats while one waits. The chat that asked waits until you answer.
- After a restart, only turns that did not finish are marked Interrupted.
- A new message, /clear, /rewind or Undo turn right after a reply ends no longer fails on a busy computer.
- Windows: Stop also ends programs that Vibe's Git Bash commands started, including ones left in the background.
  A program started in the background inside one command now ends with that command, as on macOS and Linux;
  servers that should keep running are started through Vibe Work's process list.
- Layout: the chat list, the chat and one side tab show side by side. When the window is too narrow for all three,
  the list folds; opening it by hand then folds the side tab. Maximizing a side tab folds the list and the chat to
  strips that bring them back with one click.
- Settings > Instructions edits your AGENTS.md file: Save writes it at once. Add my terminal's AGENTS.md adds that
  file's text below yours.
- Tools > Marketplace: plugins on this computer show one line with what they contain; click it for the list.
- macOS: Browser and App preview pages stay below their toolbar, so the address bar and menus take clicks.
- F5 and Ctrl/Cmd+R reload the page shown in the Browser tab instead of the app window; in the Terminal tab they
  go to the terminal. If the app window reloads anyway, its pages wait in their tabs instead of staying on screen.
- Permission cards reveal focused controls in small windows; docked notifications keep Dismiss visible.
  Native confirmations show chat names as typed.
- Skip cancels a question without counting as No.
- Rate limit this minute follows the active key and updates from its model responses.
- Delete removes the chat. Copies left on disk appear in the orphan copies list with Throw away.
- The chat list stays available when one chat’s parent cannot be read.
- Document read failures name the missing file, unsupported document or folder boundary, or a folder given
  instead of a file.
- Stop ends a chat's subagent work and waiting permission cards, including after the parent finishes its turn.
  Subagent chats open quietly and stay readable. All child cards appear in
  the parent chat and Needs you. The bell follows waiting cards, a subagent's included; internal tool
  receipts stay out of the conversation.
- Secret commands follow each secret’s current permission: allowed secrets run without asking; otherwise choose
  Deny, Allow once or Always in a short window with the secret names, program, folder and chat. Always allows
  the requested secrets for this project. User only secrets stay hidden
  from the agent. Changes in Settings > Project secrets apply immediately to open chats.
- Secret command status remembers already reported outcomes. Permission decisions stay in Settings history.
- Identical pending Git commands share one permission sheet.
- Access-mode changes stay responsive while Vibe refreshes its configuration.
- Loops stay with the chat that created them when you use /new. The strip explains that /new pauses that chat's
  loops until it is reopened. Deleting a loop updates that chat's loop label and leaves other chats' loops alone.
- Provider errors offer a plain explanation and details with stored secrets removed.
- Secret commands on Windows use the agent shell’s tool folders.
- Thinking Off turns off the reasoning response on supported Mistral models. The picker offers the levels the
  model takes; a level kept from earlier settings that the model does not take is sent as the nearest one it takes.
- Vibe 3: clearer file-search results, exact edit previews, compaction progress and the selected model's context
  limit. Vibe can wait for permission answers and secret commands, then continue when they are ready.
- Stop during compaction: the composer now says that Vibe finishes compacting in the background and that your next
  message is sent after that. Tools shows a connector that needs no sign-in as Not connected, with the reason.
- Model and thinking choices stay with each chat when you reopen it. New chats use the default in Settings.
  If a chat's chosen model is unavailable or Vibe would run a different model, it falls back to the Settings
  default with one alert naming both. The old choice is dropped and the chat stays open and sendable.
  If an agent or administrator pins another model, or the Settings default is unavailable too, the alert says
  which model Vibe runs. Each chat's model menu follows that chat's available models.
  Model menus and model-change dividers use Vibe's display names and catalog aliases.
- Plan on a page remains Vibe Work's planning mode. Chats saved in Plan first reopen in Ask with an explanation.
- Settings > Network tests company certificates with Vibe's Python connection rules too. Tools explains when a
  shared-only key or managed settings limits the available connections.
- Session logs: a secret found in a chat saved by Vibe 3 is shown with Open chat; deleting that chat removes it.
  Chats saved before the update still offer Redact. The Redact found secrets automatically switch is gone: nothing
  is changed in a saved chat until you press Redact.
- Settings > Models and agent no longer shows the Sampling (top p) and Longest answer rows.
- Branched chats can be deleted with the usual confirmation. If Vibe refuses to delete an orphaned subagent
  chat, its error is shown and the chat stays saved. The list names branched and subagent chats separately.
  Security distinguishes a recorded No, an unanswered prompt and an answer from the Debug API.
- The CLI sharing switch changes without a confirmation dialog. Its help explains the two chat lists;
  chats in the other list stay hidden until you switch back. Their copies stay with them; opening a remembered
  chat shows a plain notice.
- Saved instructions reach an open chat’s next turn once current work finishes, with the conversation kept.
  Project instructions show folder trust and offer Trust this folder before starting a new chat. The editor
  keeps its notice up to date when folder trust changes.
- Chat deletion finishes removing saved files, and retries unfinished removal the next time Vibe Work starts.
- Vibe checks HTTPS certificates with your system's certificate store, so it follows the trust decisions of your
  operating system and accepts company certificate authorities installed there.
- Updating from 0.1.1 installs Python 3.14 and Vibe 3.0.3 on first start. Vibe 3.0.3 reads older sessions; Vibe 2.26.1
  and earlier Vibe 3.0 releases cannot read sessions saved by Vibe 3.0.3, including sessions shared with your own
  command-line installation.
- Confirmation windows support the keyboard and desktop accessibility controls, with Cancel, Don't allow or Keep
  selected first.
- Changes to Vibe's settings, skills and agent profiles can be proposed for your review: the card shows each file's
  changes before you allow them. The bell groups refusals and permission decisions, with links back to the chat.
- Debug API folder choices save the requested main folder. Click outcomes report refusals, and control and
  rectangle reads follow the visible page and focused field, including buttons revealed on hover.
- Right-click menus: files, folders, search results, saved versions, editor tabs and changed lines offer actions in
  the same order. Point Vibe at this also works on selected terminal output, web pages and handbook sections;
  select words in Vibe's answer to Quote in reply.
- Send to Vibe Work: turn on Right-click menus in Settings > General > Send to Vibe Work, then right-click a file
  or folder in your file manager. On macOS, selected text also has Services > Ask Vibe Work. Items go into the
  message box; nothing is sent until you press Send.
- Skills: Tools > Skills lets you browse, install, turn on, turn off and remove skills. Skill and Marketplace
  installs show their source, licence and what runs before your Install click; removing an MCP server asks first.
  When a Mistral key is needed, Skills links to Account settings.
- GitHub reads: Settings > General > Let Vibe read GitHub with your gh login lets Vibe read pull requests, issues
  and checks using a GitHub sign-in you already set up with the GitHub command-line app. The token stays outside
  the agent, and this connection refuses writes.
- Capture: the Browser tab's Capture menu can save a page as a PDF with links or one HTML file with its pictures
  and styles. Design captures include transition timing in tokens.json. Full-page pictures keep their detail when
  App preview is zoomed out on macOS and Linux.
- Documents are read automatically on attach, including before the first message in New chat. Word,
  PowerPoint, Excel, CSV and text are read locally; PDFs and document images use OCR. Extracted text goes
  with the message. Long excerpts report which pages or lines are included and where more text is available.
  Failed reads show a reason and Try again.
- Reading a document in the chat's folder needs no card in Full access or Accept edits. Ask offers Allow once
  or No; the card names the file and page count when known, and says when its pages go to Mistral OCR.
  Allow once attaches and reads that document.
- Windows: file tools accept Git Bash drive paths, and commands that name them (`/c/Users/...`) get the same folder
  protections as native Windows paths, also when written with forward slashes (`C:/Users/...`) or in another
  letter case.
- Plugins: Remove and Turn off also end a plugin's approval when Vibe Work's data folder is reached through a link
  or spelled in another case.
- Read aloud on request: ask Vibe to read its last reply aloud once, with Skip and Stop, using your Narration voice
  without switching on voice chat or automatic narration.
- Power: Settings > General > Power > Keep this computer awake while Vibe is working is on by default. The screen
  can still turn off and lock; automatic sleep resumes when every chat is finished or waiting for you.
- The vault keeps its last state: unlocked when Vibe Work quits, it opens unlocked at the next start and after an
  update; Lock keeps it locked until you unlock it. The key stays in this computer's credential store while the
  vault is unlocked. The Remember on this computer choice is gone.
- After updating from 0.1.1 or 0.1.0, a vault that had Remember on this computer on starts locked once: type its
  password one time to establish the state it will keep for future starts.
- Settings > Account > Lock the vault when Vibe Work quits (off by default) locks the vault at every quit and deletes
  the kept key. If you had turned Remember on this computer off, this switch starts on.
- Updates check the required macOS version, Windows build or Linux system library (glibc) before downloading. If
  your system is too old, Settings > Updates says what is needed. Copies installed from a .deb point to the download
  page. Updates also clean up old installer files and keep the Settings buttons in view. An update installs only
  when its signature names the version that was offered.
- Many smaller improvements: long pastes fold into a Pasted text chip you can open; chat and side-panel headers
  line up; handbook answers take one line until you choose Show answers; warnings stay in their corner; the Full
  access warning appears when you raise access; setup accepts a replacement for a rejected key with fewer steps;
  removing secrets from saved chats handles linked folders and Windows paths; Windows plugin setup waits for busy
  folders; Windows handles very long folder paths; deleting a chat also removes its subagent chats; Linux can start
  with a warning if its desktop accessibility service is unavailable; a message queued while the agent works comes
  back to the message box once, unsent and with its pictures and context, when you press Stop or the turn fails;
  a plugin you turn on reaches chats that used subagents; the Debug API reaches the Files tab and editor controls;
  folder protection messages start with the reason, then the folder; a chat that waits for your answer sends no
  Finished notification until you have answered; and the key test in setup and Settings confirms with a short "Key works."

## 0.1.1 (2026-10-17)

Built for Vibe 2.26.1. This release adds a code editor, a Git tab for the whole project and company-network settings.

- Code editor: open a file from the chat and edit it next to the conversation, with find, undo and Save
  (Ctrl/Cmd+S). If the agent changes the same file, your unsaved edits stay: Compare or Reload.
- Git tab: the project folder, its subfolders and every session copy in one view, each with its branch, changed files
  and next step (Commit, Publish branch, Bring into main).
- File names in Vibe's answers open the file, also from subfolders. Right-click one to open, edit, show it in its
  folder or copy its path.
- Version history for beginners: a folder without git offers "Turn on version history"; set your commit name and email
  in Settings > General or in the Commit dialog. Your global git settings stay as they are.
- Company networks: Settings > Network sets a proxy, the addresses that skip it and a company certificate, with a
  connection test.
- Session logs: an optional check (Settings > Project secrets > Session logs) finds your stored secrets in saved chats
  and offers to remove them.
- Plugins you put in Vibe Work's plugins folder yourself stay off until you turn them on. The Tools page lists every
  plugin Vibe finds, with Turn on, Turn off and Remove.
- Recurring prompts: Vibe's /loop in the / menu; the chat's loops are listed and can be removed.
- Handbook pages take your answers right in the page, and a page can re-sort itself from them.
- Plainer screens: approval cards ask "Allow this edit?" or "Allow this command?"; git buttons use plain words and say
  why they are disabled; long confirmations are shown part by part.
- Windows: chats run without console windows; helper programs run hidden.
- Many smaller improvements: the message box grows up to eight lines, subagents' edits and approvals show in the
  parent chat, starting Vibe Work again brings its open window forward, and more.

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
