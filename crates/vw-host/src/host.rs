//! `Host`: starts and owns the core and the Debug API for the running app.
//!
//! Depends on: vw-core-api (`Core`), vw-debug-api (`DebugApi`), vw-supervisor, `bridge`, `forward`, `settings`.
//! Called by: `app` ([`Host::execute`], [`Host::subscribe`], [`Host::set_debug_api`], bridge), the test host.
//! Invariants:
//! - [`Host::start`] always returns a `Host`. When Vibe cannot be started the host records why in
//!   [`Startup`] and every command fails with `serverUnavailable` carrying that reason, so the
//!   window can open and say what is wrong instead of not opening at all.
//! - Every window command runs with `Origin::Ui` ([`Host::execute_window`]). The webview holds no Debug API
//!   authority: a command runs with `Origin::DebugApi` (which acts as the user without native dialogs) only when it
//!   matches a grant that a pending Debug API `ui/*` request got in Rust from its own route and body
//!   ([`crate::ui_grants`]), once, while the Debug API is on. Nothing the window sends (a flag, a token, an id) can
//!   claim that origin. While a request is served the core attributes window commands to the Debug API and holds
//!   them to its vault rule ([`UiBridge::driving`]); that only restricts.
//! - The Debug API is on in development builds and off in user builds unless the persisted
//!   setting says otherwise; turning it off removes the socket and the descriptor.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tracing::{info, warn};
use vw_core_api::{
    Command, CommandResult, Core, CoreConfig, CoreError, CoreEvent, ErrorCode, Origin,
};
use vw_debug_api::{DebugApi, DebugApiConfig, UiRoutes};
use vw_platform::{Notifier, reveal_folder};
use vw_supervisor::{SecretProvider, SupervisorConfig};

use crate::bridge::UiBridge;
use crate::confirm::SharedConfirmer;
use crate::events::{HostEvent, HostEvents};
use crate::forward::Forwarder;
use crate::notify::{NotifyRefusal, NotifyRequest, RateLimit, prepare};
use crate::settings::HostSettings;

/// Default for how long a `ui/*` route waits for the UI to answer.
pub const DEFAULT_UI_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// The `ui/*` routes this block registers: `(name, description)`. The UI registers the matching
/// handlers (`ui/src/state/uiControl.ts`); the private end-to-end test posts each one.
/// The `ui/*` routes of the Changes tab (`ui/src/features/changes/changesRoutes.ts`; `git`,
/// `ui/src/features/git/gitRoutes.ts`): `(name, description)`.
const CHANGES_ROUTES: &[(&str, &str)] = &[
    (
        "git-browser",
        "The project's Git tree. Body: {action: read|click, projectId?, nodeId?, target?: primary|commit|push|pr|bring|new-chat|new-task|file, path?}. Read returns discovered repositories, copies and changed files with state words, primary action and disabled reasons. Click validates node and file identities and reuses the existing guarded actions. New chat/task only prepares the composer; the folder of the node named by nodeId counts as chosen (like ui/worktrees folder). A push runs with the Debug API origin: its native confirmation is answered as yours.",
    ),
    (
        "changes",
        "Read the Changes tab of the shown session and move around in it like a user. Body: {action: state|reload|select|layout|comment|removeComment|sendComments, path?, layout?: split|unified, line?, endLine?, side?: new|old, text?, send?, id?, ids?}: state = mode (none|git|effects|unavailable), whether the tab is offered, the files with counts, notes and turn labels, the shown file's hunks (ids), turns (folders without git), the open confirmation, the diff comments; select shows a file; comment {path, line, endLine?, side?, text, send?} comments on lines of the current file (send puts a context chip into the composer, never a command); removeComment {id}; sendComments {ids?} puts the unsent comments into the composer as chips; layout switches split/unified; reload reads again. Keep and revert are not offered here: use POST /commands with {type: changes} (origin debug-api).",
    ),
    // UI route (`ui/src/features/git/gitRoutes.ts`, registered with the Changes tab).
    (
        "git",
        "The branch bar, the git dialogs and the PR card of the shown session, like clicks. Body: {action: state|reload|selectRepo|openCommit|setMessage|toggleFile|runHooks|commit|push|openPr|setPr|createPr|refreshCard|sendChecks|sendReview|openBring|bringIn|askVibe|close, sessionId?, repo?, text?, path?, on?, then?: none|push|pr, remote?, title?, body?, base?, draft?}: repo (with any action) first selects one of the repository roots in `repos` (for example `sub`; any other value is invalidParams), and every action then uses the selected repository; selectRepo {repo} only selects it. state (default) returns the selected repo, the discovered repos, whether the bar shows, the status (branch, base, upstream, remotes, files to commit, blocker), the PR card, the open dialog (commit: message, where it came from, files with included, author, hooks, runHooks, credit; pr: the plan, title, body, base, draft; bring: the dry run), busy, the last notice and the last commit's output. Push, createPr and bringIn carry the Debug API origin: the core's native confirmation is answered as yours; sendChecks, sendReview and askVibe put a prepared message into the composer as a chip (never sent).",
    ),
];

/// The Project tab's routes and the Plan tab's, registered by the tabs themselves (`ui/src/features/pages/register.tsx`),
/// and the Projects overview's route (`ui/src/features/projects/register.ts`). `pages` is the Project tab route's name
/// before the rename, kept as an alias for one release (v0.1.x).
const PAGES_ROUTES: &[(&str, &str)] = &[
    (
        "plan",
        "The Plan tab (the plan checklist beside the chat) of the shown session. Body: {action: state|open, step?}: state (default) returns the current plan page (path, title), its steps (id, status, text), progress (done, total, doing) and whether the tab is open; open {step} does what a click on a step does (the Project tab at that step of the plan page).",
    ),
    (
        "project",
        "The Project tab (the project handbook) of the shown session. Body: {action: state|hub|open|plan|back|forward|reload|pick|comment|send|delete|point|answers|answer|sendAnswers|check|export, ...}: open {path, anchor?} (anchor: a section or step id to scroll to); comment {path, section, text, quote?, send?}; send {id}; delete {id}; point {source: terminal|preview|page} (\"Point Vibe at this\": a context chip in the composer, never a command); answers {path?} (the page's inputs with the user's answers; every page with inputs without path and no page shown); answer {path?, input, value} (value: {type: choice, option} | {type: checklist, checked: [...]} | {type: rank, order: [...]} | {type: text, text} | null to clear; checked against the page file); sendAnswers {path?} (every answer of the page into the composer as one chip, never a command); check {path?} (the visual check now: desktop 1280 and phone 390 px, light and dark); export {path?, destination?} (the page as one self-contained HTML file: its text, or written to the absolute destination like the save dialog). path defaults to the shown page. `pick {section}` picks the named section of the shown page at once and `point` names its element with `target` ({selector} or {x, y}, source preview) or its `section` (source page): a Debug API request has no pointer, so each names what a person would have clicked, and without it the answer is invalidParams naming the missing field. Its calls are the window's, recorded as the Debug API's; an export with destination is written without the save dialog. Returns the tab state (hub with manifest, findings counts and groups, or page with steps, sections, comments, answers, visual check, prototype; the viewer) and the action's result.",
    ),
    (
        "pages",
        "Alias of ui/project for one release (the Project tab was called Pages in earlier builds). Same body and answer.",
    ),
    (
        "projects",
        "The Projects overview: every project's handbook manifest, active plans and health without opening a session. Body: {action: state|open|close|refresh|select|filter|query|openHandbook|start|migrate|newSession|reveal|chooseFolder|newProject, ...}: a project is named by projectId (or project: its main folder); select {projectId|null}; filter {status: all|active|maintenance|paused|archived}; query {text}; openHandbook {projectId, path?} (a session's Project tab); start {projectId} sends the handbook prompt as a user message in the project's current chat (or a new one), leaving a composer draft untouched; migrate {projectId} starts a session with a fixed migration prompt (the agent writes the files); newSession {projectId, folder?} starts a session in the project's main folder and selects it, sending nothing (folder: a path, needed for a project without a main folder, else invalidParams); reveal {projectId} shows the project's main folder; chooseFolder {projectId, folder} sets the main folder (a path, or null to clear; folder is required, because the native folder picker cannot be answered through the Debug API); newProject {name} creates a project. Returns the page state (cards with name, state, status, plans, health).",
    ),
];

/// The Browser tab's agent-browser route.
const BROWSER_ROUTES: &[(&str, &str)] = &[(
    "browser",
    "The Browser tab's agent browser of the selected (or given) session, every action the tab has. Body: {action: state|tabs|openTab|switchTab|closeTab|stop|resume|annotate|cancel|note|remove|send|recheck|capture|address, sessionId?: string, ...}: state = the open URL, web tabs and selected tab, whether notes are offered (the app preview only: pages of the session's own local servers), the agent strip (agentActive, inFlight, stopped), the action log (who: agent|debugApi|user, action, target, outcome, detail), the collected notes (id, mode, role, name, route, note, status draft|sent|changed|unchanged|missing, source hint, whether a crop exists), a running pick and the last error; stop / resume = the strip's buttons (Stop cancels the agent's browser action in flight and refuses more until Resume or the session's next message); annotate {mode: element|box, target} annotates the element or area `target` names and answers the new note at once (a Debug API request has no pointer: target is {ref}, {selector}, {x, y}, or {rect: {x, y, width, height}} in box mode; without it the answer is invalidParams naming the missing field); cancel stops a pick; note {id, text}; remove {id}; send puts every draft note into the composer (a context chip and a cropped picture each, never a command) and answers how many; recheck {reload?: bool} reloads the preview and sets each sent note to changed / unchanged / missing; tabs lists web tabs; openTab {url?} opens one; switchTab/closeTab {tabId} act on a tab; capture {kind} starts a Browser capture; address {url} opens a URL like the address bar (the same navigation policy as the agent's).",
)];
const SHELL_ROUTES: &[(&str, &str)] = &[
    (
        "controls",
        "Read visible main-window controls or the elements a selector names: testId, itemId, label, text, value, checked, pressed, expanded, selected and disabled. Secret fields are withheld; selectors run with their values emptied. Native pages and editor shadow controls use their typed routes.",
    ),
    (
        "walkthrough",
        "Debug walkthrough caption. Body: {action: read|start|step|pause|finish|stop, total?, step?, label?}. Escape pauses pacing; finish hides the caption after three seconds.",
    ),
    (
        "trace",
        "Read recent UI command metadata through GET /ui/trace?since=sequence. Returns entries, latestSeq, oldestSeq and truncated; never command input or results.",
    ),
    (
        "secrets",
        "Where the secret fields (key and password fields) are on screen. Body: {lock?: bool}: true turns the capture lock on (fields show dots, Show is disabled), false answers and then releases it (GET /screenshot does both around a capture). Returns {rects, clear, locked, epoch, viewportWidth, viewportHeight} in CSS pixels, never a value.",
    ),
    (
        "click",
        "Clicks an element like the mouse does (every control is reachable, also those without a route of their own). Body: {selector: CSS selector, index?: n-th match}. A hidden or disabled element is refused (denied). Answers right after the click; an action that waits (a native folder picker) keeps running. The click is the window's: a native confirmation it leads to opens as the window's (GET /asks lists it, POST /asks/{id} answers it; a vault confirmation waits for a person). Routes of their own answer their confirmations as yours.",
    ),
    (
        "key",
        "Presses a key like the keyboard does. Body: {key: e.g. Escape|Enter|k, selector?: CSS selector to focus first, ctrl?, shift?, alt?, meta?}. Without a selector the key goes to the focused element. Like ui/click, a native confirmation it leads to opens as the window's (POST /asks/{id}).",
    ),
    (
        "menu",
        "The app's action menus (right-click menus, the menu key, Shift F10 and a row's ⋯ open the same menu). Body: {action: state|open|choose|close, selector?, index?, item?}: state (default) = the open menu or {open: false}: its testId, label, heading (the line naming the target: a path, path:line, a commit) and items (id, label, keys as shown, disabled, danger) in the order shown; open {selector, index?} right-clicks that element like the mouse does (a hidden element is refused) and answers the menu it opened; choose {item} runs an item exactly as a click does (a disabled item is refused; a native confirmation it leads to opens as the window's, POST /asks/{id}) and closes the menu; close = Escape. The menu key is ui/key {key: ContextMenu} or {key: F10, shift: true}; a row's ⋯ is ui/click.",
    ),
    (
        "palette",
        "The command palette, like the keyboard uses it. Body: {action: list|open|close|run, query?, id?}: list (default) returns the commands (id, label, section) the query matches; open/close show or hide it; run {id} runs one exactly as Enter does (like ui/click: a native confirmation it leads to opens as the window's, POST /asks/{id}).",
    ),
    (
        "toasts",
        "The toasts on screen. Body: {action: read|dismiss, id?}. Returns the list (id, kind, text).",
    ),
    (
        "revision",
        "The UI mutation counter (like ShellX's uiRevision). No body. Returns {revision, quietForMs, animations}: read it before an action and wait until it moved and the UI is quiet before a screenshot.",
    ),
    (
        "settle",
        "Waits until the UI is quiet. Body: {quietMs?: 250, timeoutMs?: 5000 (max 30000)}: no DOM mutation for quietMs, no running animation, two frames painted. Returns {revision, settled, waitedMs}; settled false means the timeout came first.",
    ),
    (
        "rect",
        "An element's rectangle (used by GET /screenshot?crop=). Body: {selector: CSS selector}. Returns {found, x, y, width, height, viewportWidth, viewportHeight} in CSS pixels.",
    ),
    (
        "layout",
        "Automatic layout checks of what is shown (the visual pass). No body. Returns the viewport, every shown menu, listbox, dialog, tooltip and popover with whether it is fully inside the window, buttons and headings whose text is cut off, horizontal scroll, and a problems list.",
    ),
    (
        "panel",
        "Open, close or toggle the side panel and pick its tab. Body: {action: open|close|toggle, tab?: project|plan|browser|git|changes|terminal|file} (pages = project, an alias for one release). Returns the panel state.",
    ),
    (
        "state",
        "Read the shell state: theme, list visibility, selected session, panel, the overlay that covers the content (settings, palette, ...), the toasts on screen, the app version (appVersion, as Settings > About shows it; null before the core answered) and the session rows with their status badges.",
    ),
    (
        "theme",
        "Set the theme. Body: {theme: dark|light|toggle}. Returns the theme.",
    ),
    (
        "list",
        "Show, hide or toggle the session list. Body: {action: show|hide|toggle}. Returns whether it is visible.",
    ),
    (
        "session",
        "Select a session, or the new-chat view (null). Body: {sessionId: string|null}. Returns the selected id.",
    ),
    (
        "rail",
        "Read or press the icon rail. Body: {action: read|home|newChat|projects|tools|libraries|settings}: read (default) returns the items top to bottom (id, label), the current one, whether a page lies over the work view, where Home goes (the selected session id and title, or null for the new-chat view) and the new-chat view's folder chip; home leaves Settings, the Projects overview, the Tools page and the Libraries page and shows the work exactly as it was (never a new session); newChat opens the new-chat view with its composer focused (no session exists until its first send, see composer); projects, tools, libraries and settings open those pages as their buttons do.",
    ),
    (
        "sessions",
        "Act on the session list like a click does. Body: {action: filter|archived|more|refresh|open|rename|pin|archive|delete|retryReopen|cancelReopen|continueNew|toggleGroup|projectCreate|projectRename|projectDelete|projectMove|projectPlace|projectStart|projectFolder|projectsRefresh, ...}: filter {text}, archived {show: bool}, more (load the next page), refresh, open/pin/archive/delete/rename {sessionId, ...} (pin {pinned: bool}, archive {archived: bool}, rename {title}); delete runs without the native confirmation (the Debug API acts as the user) and also answers leftRunning: the processes the deleted session left running (they are not stopped, use ui/processes stop). retryReopen/cancelReopen/continueNew handle a session stuck reopening; projectCreate/Rename/Delete/Move/Place/Start/Folder and projectsRefresh manage projects. temporaryChat {on: bool} sets the new chat's \"Temporary chat\" switch. Returns the list model (groups, pinned, needs-you, unread flags, badges), the temporary session ids as `temporary` and the switch as `newChatTemporary`.",
    ),
    (
        "transcript",
        "Read or act on the transcript like a click does. Body: {action: read|open|answer|protected|grants|grantsOpen|revokeGrant|grantsAllowed|link|sources, sessionId?: string (default: the selected session)}: read returns the rows as rendered (kinds, keys, entry ids, a card's question, buttons with their keys and covers words, web sources); open {key, open?: bool} opens or closes a group, step or show-all; answer {callbackId, choice?: once|session|always|no, decision?: approve|approve_for_session|deny, feedback?, answers?: [{labels: [string], other?: string}], cancel?: bool} answers a card with a button it shows (Vibe Work keeps a session answer itself and sends Vibe approve; no Always allow); protected {proposalId, choice: once|no, note?}; grants, grantsOpen {open?}, revokeGrant {id}, grantsAllowed drive the session header's grants chip and list; link {url} shows the link dialog; sources {key, open?: bool} opens or closes the side list of an answer's web sources.",
    ),
    (
        "composer",
        "Read or act on a session's composer like typing and clicking do. Body: {action: read|type|send|insertContext|attachImage|stop|retry|slash|access|model|queueEdit|queueRemove|rewind|rewindClose|branch, sessionId?: string (default: the selected session, else the new-chat view's composer \"new-chat\")}: type {text}; send {mode?: send|steer|queue} sends the current content (a typed !cmd runs a shell command, a GUI /command runs it; on \"new-chat\" it first starts the session in the folder chip's folder, applies the held access and model, then sends the first turn, and outcome.started names the new session; without a folder it refuses and keeps the text); insertContext {label, source, text} adds a context chip (always data, never a command); attachImage {name, data: base64}; retry {instructions?}; slash {name, args?}; access {mode} and model {alias?, effort?} answer with the composer as it then reads (on the new-chat composer they are held until its first send); queueEdit {queueItemId, text}; queueRemove {queueItemId}; rewind {entryId, confirm?: here|new}; branch {entryId}.",
    ),
    (
        "setup",
        "Read the Mistral key screens. No body. READ-ONLY: returns whether a key service exists, whether the setup screen covers the content, the step (create, pickVault, unlock, key, ready, blocked), the vault and key states, the last test outcome and failure reason word, and counts of saved keys and Vibe's own key places. Never a key or a password; changes go through the `keys` command.",
    ),
    (
        "settings",
        "Read or act on the Settings page like clicks do. Body: {action: read|open|close|category|search|statusUsage|instructions|errorDetail|cliSharing}: read (default) returns whether the page is open, the category list and the shown category, the search inside Settings and its results (id, category, label, path), the shown category's rows as the user sees them (id, label, value: a switch's state, a pressed segment or a selector's choice, lit), the About facts (version, author, contact mailto: target, source link target), the personal instruction editor (document facts, draft text, dirty, review step, busy, notice, error), the trust of the session folder and the error-detail preference; cliSharing {share?: bool} saves the CLI session-sharing switch when `share` is given and returns it with the CLI home and both Vibe versions (applies at the next start); open {category?, setting?} shows the page at a category (with `setting`: at that setting, lit for a moment), close hides it (the view underneath shows again unchanged); category {id} shows one category; search {query} types into the search inside Settings; statusUsage {show?: bool} reads or sets the Appearance switch for the status bar's context readout; instructions {do: load|type|review|back|discard|save|loadCli, scope: personal, text?} edits the personal AGENTS.md through the same review step the screen has (save writes the reviewed draft only, and then makes open sessions reopen with it); errorDetail {mode: plain|details}.",
    ),
    (
        "documents",
        "Read or act on the document chips like clicks do. Body: {action: read|extract|remove|preview|asPicture, sessionId?: string (default: the selected session), docId?: string}: read returns each session's document chips (id, name, kind, status, pages, chars, kept, error) and the open preview; extract {docId} reads it with Mistral OCR (spends OCR quota); remove {docId}; preview {docId, open?: bool}; asPicture {docId} puts a PNG/JPEG/WebP/GIF into the message as a picture. To add a file, use ui/files drop (the same routing as a drop on the composer).",
    ),
    (
        "voice",
        "Read or drive dictation, narration and voice chat like the mic button, the narration bar and the Voice chat button do. Body: {action: read|start|stop|cancel|readAloud|narrateLast|stopNarration|skipNarration|settings|voiceChat|voiceChatEnd, sessionId?: string (default: the selected session)}: read returns the dictation state (state, streamId, mode, seconds, live text, last inserted text, error, micPending, permissionWait), narration (enabled, playing, queue, last played: chars, bytes, seconds) and voiceChat (on, owner, phase: off|listening|sending|thinking|waiting|speaking|error, micPending, partial, heard, endedBy: button|pause|limit, lastSpoken {text, chars, seconds, kind}, spoken, note, error, autoEnd, pauseMs, latency); start turns the microphone on and answers once it records, failed, or waits for the microphone permission (micPending; an OS prompt has no time limit, poll read); stop inserts the text at the caret of the session's text box (marked as text from an external client); readAloud {text?} reads the last finished reply or the requested text once, without changing voice settings, and returns before audio finishes; narrateLast narrates the session's last finished turn; settings {narration?, voice?, language?, micId?, voiceChatAutoEnd?, voiceChatPauseMs?} changes the Settings screen's voice choices; voiceChat {on: true|false} turns voice chat on for the session (moving it from another session) or off; voiceChatEnd ends the user's turn now, like pressing the button while it listens.",
    ),
    (
        "connectors",
        "Read or act on the Connectors page like clicks do. Body: {action: read|open|close|filter|search|select|refresh|signIn|stopWaiting|turnOff|turnOn|addServer|signInServer|signOutServer|removeServer|serverOff|serverOn|addKeyServer|allowServerKey, alias?, name?, url?, secretId?, newSecret?, header?, filter?: all|connected|signin|off, text?}: read (default) returns whether the page is open, the filter, the groups with each connector's status, the servers (a key server with its key: the secret's name, the header and the grant, never a value), the sign-in in progress (connector or server, the page address, waiting) and the selection; signIn {alias} starts a Mistral connector sign-in through the selected (else newest) session and opens the link dialog with the page address; addServer {url, name?} adds a server that signs in with OAuth (refused addresses are refused before any dialog); addKeyServer {url, name?, secretId | newSecret {name, value}, header?} adds a server that needs an API key and allowServerKey {name} allows its key again: these two always show the native dialog in the window, also for the Debug API, and the user's answer decides. A newSecret with a value, and removeServer of a key server, are the user's alone: refused here (403 vaultUserOnly). Other changes run without the native confirmation (the Debug API acts as the user); sign-ins finish only in a browser.",
    ),
    (
        "marketplace",
        "Read or act on the Tools page's Marketplace tab like clicks do. Body: {action: read|open|connectors|reload|filter|search|select|install|offer|installOffer|cancelOffer|cancel|remove|addServer|turnOnPlugin|turnOffPlugin|removePlugin, id?, path?, filter?: all|installed|updates, text?}: read (default) returns the tab, filter, selection, note, the install in progress, the platform, every shown entry (plugins, skill packs, MCP servers) with its status words, installed version, update flag, access class for servers and its state in the viewed session, and discovered: the plugins outside the catalog (name, path, scope, managed, off, components, state in the viewed session); install {id} installs or updates a pack or plugin from the reviewed, pinned catalog without an offer (runs in the background; follow the marketInstall events); offer {id} is the card's Install… (selects the card and shows the install offer in the detail: name, version, publisher, from, licence, what it runs), installOffer {id} is the detail's Install (installs exactly the shown offer, or shows the offer first when none is shown; a stale offer is answered with a fresh one and the notice, never installed by itself) and cancelOffer {id} closes it; read lists them under offers (phase, offerId, facts with their Changed marks, confirm words, notice, read failures); cancel stops it, remove {id} removes it, addServer {id} adds a catalog MCP server through the Connectors flow (sign-in finishes in a browser); turnOffPlugin, turnOnPlugin and removePlugin {path} take the exact path from discovered and change only plugin folders Vibe Work may change (managed): turn off moves the folder to plugins-off/ and turn on moves it back (a plugin of the Vibe home loses its approval on turn off and gets it on turn on), remove deletes the folder; open chats reload their plugins. Changes run without the native confirmation (the Debug API acts as the user).",
    ),
    (
        "skills",
        "Read or act on the Tools page's Skills tab like clicks do. Body: {action: state|open|reload|search|disclose|install|update|confirm|cancel|enable, name?, text?, enabled?}: state (default) returns whether the tab shows, loading, the list error, note, search, busy, each row (name, group installed|catalog, version, latestVersion, updateAvailable, skillId, enabled, locked, expanded, error) and offers (each open install offer: target, mode install|update, phase reading|shown|installing, offerId, expiresAt, heading, facts with label and value, confirm words, notice and noticeKind changed|ranOut; read failures; the focus request); open shows the tab; disclose {name} opens or closes a row; install {name} is a catalog row's Install… and update {name} an installed row's Update to vN…: both show the offer (what the click installs); confirm {name} is the offer's Install vN / Update to vN and sends that offer's id (a used or changed offer is refused as staleOffer, an expired one as offerExpired: the fresh offer shows like any offer with one sentence above the buttons, This offer changed and was refreshed. Check the details above, then install again. or This offer expired and was refreshed. Check the details above, then install again. (an update offer ends then update again.), and is never sent by itself); cancel {name} closes it; enable {name, enabled} is the row's switch. No native dialog (the Debug API acts as the user); Remove keeps its window dialog (ui/click).",
    ),
    (
        "libraries",
        "Read or act on the Libraries page like clicks do. Body: {action: read|open|close|select|create|scope|delete|upload|deleteDocument|text|closeText|ask, libraryId?, documentId?, name?, scope?: off|global|project:<id>, files?: [{name, data (base64)}], question?}: read (default) returns whether the page is open, the libraries (id, name, documents, scope: who may use it in chats), the sentences about what is sent and what Mistral charges, the selected library's documents (id, name, Mistral's processing status, pages), the note, the last answer with its numbered sources and the open document text (its start); create {name, scope?} makes a library on Mistral (default scope global); upload sends the files' bytes to Mistral (like a drop; there is no path action, the window's Add files uses the native picker); delete deletes the library and its documents on Mistral; ask {question} asks across every library that is not off (spends Mistral calls). No native dialog (the Debug API acts as the user).",
    ),
    (
        "notifications",
        "Read or set the notification state. Body: {action: read|level|focus}: level {level: errors|permissions|all}, focus {focused: bool|null} overrides the window-focus signal for tests (null = real). Returns the level, the focus override and the OS notifications the UI asked for since the page loaded.",
    ),
    // UI route (`ui/src/features/updates/updatesRoutes.ts`, registered by the shell wiring).
    (
        "processes",
        "The \"Left running\" list: processes the agent started that keep running after their session was deleted. Body: {action: read|open|close|stop, id?}: read (default) returns {open, items: [{id, sessionId, sessionTitle, pid, name, ports}], stopping}; open/close show or hide the list dialog like the status bar button and Close do; stop {id} stops one listed process like its Stop button (no dialog: the request is the click).",
    ),
    // UI route (`ui/src/features/search/searchRoutes.ts`, registered by the shell wiring).
    (
        "search",
        "Search like Ctrl K and Ctrl F do. Body: {action: palette|results|run|find|next|previous|findClose|fileFind|fileNext|filePrevious|fileFindClose|findState, query?, id?, sessionId?}: palette {query} opens the global search with the words typed; results (default) returns the rows the palette shows now (id, kind: session|message|file|setting|action, title, path, opens); run {id} opens one like Enter (a message opens its conversation at the match with the find bar, a file opens the File tab at the line); find {query?, sessionId?} is Ctrl F in a conversation (the find bar; sessionId defaults to the open session), next/previous step through its matches, findClose closes it; fileFind/fileNext/filePrevious/fileFindClose do the same in the File tab; findState returns both bars (session, query, current match) and their match counts.",
    ),
    (
        "models",
        "The card that asks for a new default model when Vibe no longer offers the Settings one. Body: {action: read|check|pick|later, alias?}: read (default) returns {open, gone {alias, label} or null, models [{alias, label}] (the models Vibe offers), saving, error}; check runs the launch check again (checkDefaultModel); pick {alias} saves that model as the default for new chats like its button and closes the card; later closes it without a change (it asks again at the next launch).",
    ),
    (
        "updates",
        "Read or act on the update flow like the notice and the Settings section do. Body: {action: read|check|download|install|autoDownload|later, enabled?: bool}: read (default) returns whether this build has an update channel, the flow's view (phase: idle|checking|available|osTooOld|downloading|ready|installing|restarting|failed, version, notes, progress, mode inApp|linkOnly, autoDownload, releasePage; osTooOld = the new version needs a newer system, nothing is downloaded: os, minimum and this computer's current version), the status line and the notice text; check asks the update server now; download and install click the same buttons as the window (no dialog either way: the request is the user's agreement); install only works for a downloaded and verified (ready) update and restarts the app; autoDownload {enabled} is the Settings switch; later hides the notice.",
    ),
    // UI route (`ui/src/features/worktrees/worktreeRoutes.ts`, registered by the shell wiring).
    (
        "worktrees",
        "Read the worktree UI. Body: {action: state|folder, path?}: state = the new-chat view's folder (newFolder, newFolderBy), the open end-of-session choice (session, why, the inspection: files a removal discards, branch plan, why it is blocked) and the copies from deleted sessions; folder {path} chooses a folder for the new-chat view like Choose folder does (pickFolder: it counts as picked for this run). Start, keep and remove go through POST /commands with {type: worktrees} (no native dialog for the Debug API).",
    ),
];

/// Whether Vibe is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// The core started and the server answered its handshake.
    Running,
    /// This build or machine has no way to start Vibe yet (no setup block, no dev server).
    Unavailable(String),
    /// A start was attempted and failed.
    Failed(String),
}

impl Startup {
    /// The reason text for the two non-running states.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Running => None,
            Self::Unavailable(r) | Self::Failed(r) => Some(r),
        }
    }
}

/// Everything [`Host::start`] needs.
pub struct HostConfig {
    /// The supervisor config, or the reason none could be built (shown in the UI).
    pub launch: Result<SupervisorConfig, String>,
    /// Where the API key comes from.
    pub secrets: Arc<dyn SecretProvider>,
    /// Core tuning (`Default` for production values).
    pub core: CoreConfig,
    /// `settings.json` (the Debug API switch).
    pub settings_path: PathBuf,
    /// Folder for the Debug API's per-run descriptor.
    pub descriptor_dir: PathBuf,
    /// Development build: the Debug API is on unless the setting says off.
    pub dev_build: bool,
    /// How long a `ui/*` route waits for the UI to answer ([`DEFAULT_UI_REPLY_TIMEOUT`]).
    pub ui_reply_timeout: Duration,
    /// Folders to register as user-picked at start. Tests only; in the app only the native folder
    /// dialog may do that ([`Host::register_picked_folder`]).
    pub picked_folders: Vec<PathBuf>,
    /// Shows the native confirmation dialogs (the app's dialog; [`SharedConfirmer::decline`] when
    /// none can be shown).
    pub confirmer: SharedConfirmer,
    /// Shows OS notifications and reports clicks (`vw_platform::DesktopNotifier` in the app).
    pub notifier: Arc<dyn Notifier>,
    /// The key service. When set, it starts before the core: the server gets
    /// its placeholder (`secrets` is replaced) and the `VIBE_*` overrides that route Mistral calls
    /// through the model proxy, and `keys` commands reach it. `None`: the key comes from `secrets`
    /// (tests and development runs with `MISTRAL_API_KEY` in the environment).
    pub keys: Option<vw_keys::KeyServiceConfig>,
    /// The app window for the Debug API's `/windows`, `/screenshot` and `/window`; `None` in runs
    /// without a window (test hosts, the browser dev UI).
    pub app_window: Option<vw_debug_api::SharedAppWindow>,
}

/// The running host. Cheap to share behind an `Arc`.
pub struct Host {
    core: Option<Core>,
    startup: Startup,
    ui: UiBridge,
    routes: UiRoutes,
    debug: AsyncMutex<Option<DebugApi>>,
    forwarder: Forwarder,
    runtime: Handle,
    notifier: Arc<dyn Notifier>,
    limiter: RateLimit,
    events: HostEvents,
    pub(crate) settings_path: PathBuf,
    descriptor_dir: PathBuf,
    dev_build: bool,
    keys: Option<Arc<vw_keys::KeyService>>,
    app_window: Option<vw_debug_api::SharedAppWindow>,
}

/// Starts the key service when configured and adapts the launch to it (placeholder, overrides).
async fn start_keys(
    keys: Option<vw_keys::KeyServiceConfig>,
    launch: &mut Result<SupervisorConfig, String>,
    secrets: &mut Arc<dyn SecretProvider>,
    core: &mut CoreConfig,
) -> Option<Arc<vw_keys::KeyService>> {
    // An unavailable key store never authorizes a custom network record.
    vw_mistral::network::set_current(vw_mistral::network::NetworkSettings::from_environment());
    let Some(config) = keys else {
        core.network_settings_rejected = core
            .network_file
            .as_deref()
            .is_some_and(|file| vw_mistral::network::load_for_launch(file, None).rejected);
        return None;
    };
    let supervisor = launch.as_mut().ok()?;
    match vw_keys::KeyService::start_with_network(config, core.network_file.clone()).await {
        Ok(service) => {
            core.network_settings_rejected = service.network_settings_rejected();
            if let Some(list) = vw_mistral::network::current()
                .child_no_proxy(&vw_mistral::network::NetworkSettings::from_environment())
            {
                supervisor
                    .extra_env
                    .push(("NO_PROXY".to_owned(), list.into()));
            }
            supervisor.extra_env.extend(service.vibe_env());
            *secrets = service.secret_provider();
            core.keys = Some(Arc::clone(&service));
            info!("key service started; Vibe gets a placeholder key");
            Some(service)
        }
        Err(error) => {
            warn!(%error, "the key service did not start");
            *launch = Err(format!("the key service did not start: {error}"));
            None
        }
    }
}

impl Host {
    /// Starts the core (when Vibe can be started) and the Debug API (when enabled).
    ///
    /// Must run inside a tokio runtime (Tauri's async runtime is one).
    pub async fn start(config: HostConfig) -> Self {
        // The startup state machine is large (about 16 KiB on the Windows target, every service's setup state):
        // boxed here once so every caller awaits a small future (clippy::large_futures with -D warnings).
        Box::pin(Self::start_inner(config)).await
    }

    async fn start_inner(config: HostConfig) -> Self {
        let ui = UiBridge::new(config.ui_reply_timeout);
        let routes = UiRoutes::default();
        for (name, description) in SHELL_ROUTES
            .iter()
            .chain(crate::terminal::TERMINAL_ROUTES)
            .chain(CHANGES_ROUTES)
            .chain(PAGES_ROUTES)
            .chain(crate::files_routes::FILES_ROUTES)
            .chain(BROWSER_ROUTES)
        {
            if let Err(error) = ui.register_route(&routes, name, description) {
                warn!(%error, route = name, "ui route not registered");
            }
        }
        if let Err(error) = ui.register_client_routes(&routes) {
            warn!(%error, "ui client routes not registered");
        }
        let mut launch = config.launch;
        let mut secrets = config.secrets;
        let mut core_config = config.core;
        if !config.settings_path.as_os_str().is_empty() {
            core_config.secret_scan_data = config.settings_path.parent().map(Path::to_path_buf);
        }
        if let Some(cap) = HostSettings::load(&config.settings_path).terminal_cap() {
            core_config.terminals.max_per_session = cap;
        }
        let keys = start_keys(config.keys, &mut launch, &mut secrets, &mut core_config).await;
        let (core, startup) = match launch {
            Err(reason) => {
                info!(%reason, "Vibe is not started in this run");
                (None, Startup::Unavailable(reason))
            }
            Ok(supervisor) => {
                match Core::start(supervisor, secrets, config.confirmer, core_config).await {
                    Ok(core) => (Some(core), Startup::Running),
                    Err(error) => {
                        warn!(%error, "the core did not start");
                        (None, Startup::Failed(error.to_string()))
                    }
                }
            }
        };
        if let Some(core) = &core {
            // Grants pin catalog facts; the core attributes window commands to the Debug API while it drives.
            ui.set_facts(Arc::new(crate::ui_grants::CoreFacts(core.clone())));
            let bridge = ui.clone();
            core.set_window_drive(move || bridge.driving());
            for folder in &config.picked_folders {
                if let Err(error) = core.register_picked_folder(folder) {
                    warn!(%error, "picked folder not registered");
                }
            }
        }
        let host = Self {
            core,
            startup,
            ui,
            routes,
            debug: AsyncMutex::new(None),
            forwarder: Forwarder::default(),
            runtime: Handle::current(),
            notifier: config.notifier,
            limiter: RateLimit::default(),
            events: HostEvents::new(),
            settings_path: config.settings_path,
            descriptor_dir: config.descriptor_dir,
            dev_build: config.dev_build,
            keys,
            app_window: config.app_window,
        };
        let setting = HostSettings::load(&host.settings_path).debug_api;
        host.start_debug_api(setting).await;
        host
    }

    async fn start_debug_api(&self, user_setting: Option<bool>) {
        let Some(core) = &self.core else { return };
        let mut config = DebugApiConfig::new(self.descriptor_dir.clone());
        config.user_setting = user_setting;
        config.dev_build = self.dev_build;
        config.ui_routes = self.routes.clone();
        config.window = self.app_window.clone();
        match DebugApi::start(core.clone(), config).await {
            Ok(api) => {
                // the Debug API's port is never reachable from a browser page.
                if let (Some(api), Some(guard)) = (&api, core.network_guard()) {
                    guard.add_internal_port(api.port());
                }
                *self.debug.lock().await = api;
            }
            Err(error) => warn!(%error, "the Debug API did not start"),
        }
    }

    /// Whether Vibe is running and, if not, why.
    #[must_use]
    pub fn startup(&self) -> &Startup {
        &self.startup
    }

    /// The core handle, when it is running (tests register folders and read status through it).
    #[must_use]
    pub fn core(&self) -> Option<&Core> {
        self.core.as_ref()
    }

    /// The key service, when this run has one (the private test host seeds a scratch vault through it
    /// during live runs; the app never needs it here).
    #[must_use]
    pub fn key_service(&self) -> Option<&Arc<vw_keys::KeyService>> {
        self.keys.as_ref()
    }

    /// The UI-control broker (the two Tauri commands call it).
    #[must_use]
    pub fn ui(&self) -> &UiBridge {
        &self.ui
    }

    /// The Debug API's descriptor path while it runs.
    pub async fn debug_descriptor(&self) -> Option<PathBuf> {
        self.debug
            .lock()
            .await
            .as_ref()
            .map(|api| api.descriptor_path().to_path_buf())
    }

    fn unavailable(&self) -> CoreError {
        let reason = self.startup.reason().unwrap_or("Vibe is not running");
        CoreError::new(ErrorCode::ServerUnavailable, reason)
    }

    /// The core, or `serverUnavailable` with the startup reason.
    pub(crate) fn core_or_unavailable(&self) -> Result<&Core, CoreError> {
        self.core.as_ref().ok_or_else(|| self.unavailable())
    }

    /// Executes one command for the app's own window (`origin = ui`).
    ///
    /// # Errors
    /// The core's [`CoreError`], or `serverUnavailable` with the startup reason when Vibe is not
    /// running.
    pub async fn execute(&self, command: Command) -> Result<CommandResult, CoreError> {
        self.execute_as(Origin::Ui, command).await
    }

    /// Executes one command the main window sent (the Tauri `core_execute`): `Origin::Ui`, unless it matches an unused
    /// grant of a pending Debug API `ui/*` request while the Debug API is on; then that one command runs with
    /// `Origin::DebugApi` and the grant is used up (module docs). Logged with the grant's name and the request id.
    ///
    /// # Errors
    /// As [`Host::execute`].
    pub async fn execute_window(&self, command: Command) -> Result<CommandResult, CoreError> {
        let granted = if self.debug.lock().await.is_some() {
            serde_json::to_value(&command)
                .ok()
                .and_then(|value| self.ui.take_grant(&value))
        } else {
            None
        };
        match granted {
            Some((request, grant)) => {
                info!(
                    command = command.name(),
                    grant,
                    request,
                    "a window command runs as the Debug API under its request's grant"
                );
                self.execute_as(Origin::DebugApi, command).await
            }
            None => self.execute_as(Origin::Ui, command).await,
        }
    }

    async fn execute_as(
        &self,
        origin: Origin,
        command: Command,
    ) -> Result<CommandResult, CoreError> {
        match &self.core {
            Some(core) => core.execute(origin, command).await,
            None => Err(self.unavailable()),
        }
    }

    /// Starts forwarding core events to `sink` (first the snapshots, then changes), replacing a
    /// previous subscription. `sink` returns `false` when its receiver is gone.
    ///
    /// # Errors
    /// `serverUnavailable` when there is no core to subscribe to.
    pub fn subscribe<S>(&self, sink: S) -> Result<(), CoreError>
    where
        S: FnMut(CoreEvent) -> bool + Send + 'static,
    {
        let Some(core) = &self.core else {
            return Err(self.unavailable());
        };
        self.forwarder
            .replace(&self.runtime, core.subscribe(None), sink);
        Ok(())
    }

    /// Records a folder the user picked in the native folder dialog (the only way a folder can
    /// become trustable) and returns the path as the dialog gave it, for the UI to use as a session
    /// folder. The webview never supplies the path: the app's Tauri command opens the dialog.
    ///
    /// # Errors
    /// `invalidParams` when the path is not an existing directory; `serverUnavailable` without a core.
    pub fn register_picked_folder(&self, path: &Path) -> Result<String, CoreError> {
        let Some(core) = &self.core else {
            return Err(self.unavailable());
        };
        core.register_picked_folder(path).map_err(|error| {
            CoreError::new(
                ErrorCode::InvalidParams,
                format!("cannot use that folder: {error}"),
            )
        })?;
        Ok(path.display().to_string())
    }

    /// Shows a folder in the file manager only when a listed or loaded session works in it or a
    /// saved project names it as its main folder. A picked folder alone is not enough.
    ///
    /// # Errors
    /// `denied` for a path no session or project uses or a network path (Windows); `internal`
    /// when the path is no existing folder or the file manager cannot start.
    pub fn reveal_folder(&self, path: &str) -> Result<(), CoreError> {
        let Some(core) = &self.core else {
            return Err(self.unavailable());
        };
        if !core.is_session_folder(path) && !core.is_project_main_folder(path) {
            return Err(CoreError::new(
                ErrorCode::Denied,
                "that folder does not belong to any session or project",
            ));
        }
        reveal_folder(Path::new(path)).map_err(|error| {
            let error_code = match error {
                // A network or device path is never opened (it would authenticate to a server).
                vw_platform::RevealError::NotLocal(_) => ErrorCode::Denied,
                _ => ErrorCode::Internal,
            };
            CoreError::new(error_code, error.to_string())
        })
    }

    /// Opens a web link in the system browser after the user confirmed it in the link dialog.
    /// The main window never navigates; the URL is checked again here (http/https only, no
    /// user name, no whitespace or control characters) before any program starts.
    ///
    /// # Errors
    /// `invalidParams` for a link that may not be opened, `internal` when the browser cannot start.
    pub fn open_link(&self, url: &str) -> Result<(), CoreError> {
        vw_platform::open_web_url(url).map_err(|error| {
            let code = match error {
                vw_platform::OpenUrlError::Spawn(_) => ErrorCode::Internal,
                _ => ErrorCode::InvalidParams,
            };
            CoreError::new(code, error.to_string())
        })
    }

    /// Opens a `mailto:` link to one plain address in the system's mail program (the contact line in
    /// Settings › About). Only `mailto:` with a single address passes (`vw_platform::mail_url`); nothing is sent,
    /// the mail program only opens a new message.
    ///
    /// # Errors
    /// `invalidParams` for any other link, `internal` when the opener cannot start.
    pub fn open_mail_link(&self, url: &str) -> Result<(), CoreError> {
        vw_platform::open_mail_url(url).map_err(|error| {
            let code = match error {
                vw_platform::OpenUrlError::Spawn(_) => ErrorCode::Internal,
                _ => ErrorCode::InvalidParams,
            };
            CoreError::new(code, error.to_string())
        })
    }

    /// Shows an OS notification for the UI and publishes [`HostEvent::OpenSession`] if the user
    /// clicks it.
    ///
    /// # Errors
    /// `invalidParams` for a bad session id, `denied` when rate limited, `internal` when the OS
    /// cannot show it.
    pub fn notify(&self, request: &NotifyRequest) -> Result<(), CoreError> {
        let shown =
            prepare(request, &self.limiter, std::time::Instant::now()).map_err(|refusal| {
                match refusal {
                    NotifyRefusal::BadSession => {
                        CoreError::new(ErrorCode::InvalidParams, refusal.to_string())
                    }
                    NotifyRefusal::RateLimited => {
                        CoreError::new(ErrorCode::Denied, refusal.to_string())
                    }
                }
            })?;
        let publisher = self.events.publisher();
        let session_id = request.session_id.clone();
        let notification_id = request.notification_id;
        self.notifier
            .show(
                shown,
                Box::new(move || {
                    let _ = publisher.send(HostEvent::OpenSession {
                        session_id,
                        notification_id,
                    });
                }),
            )
            .map_err(|error| CoreError::new(ErrorCode::Internal, error.to_string()))
    }

    /// Starts forwarding [`HostEvent`]s to `sink`, replacing an earlier forwarder.
    pub fn subscribe_host<S>(&self, sink: S)
    where
        S: FnMut(HostEvent) -> bool + Send + 'static,
    {
        self.events.forward(&self.runtime, sink);
    }

    /// Delivers an OS update-notification activation to the UI.
    pub fn open_updates(&self) {
        let _ = self.events.publisher().send(HostEvent::OpenUpdates);
    }

    /// Delivers a native browser context-menu click as metadata only: Save to vault (the core still consumes the
    /// choice and requires native confirmation before storing anything) or, with `point`, Point Vibe at this.
    pub fn browser_context_menu(&self, session_id: String, point: bool) {
        let event = if point {
            HostEvent::BrowserPointContext { session_id }
        } else {
            HostEvent::BrowserSaveContext { session_id }
        };
        let _ = self.events.publisher().send(event);
    }

    /// Persists the Debug API switch and applies it now: starts the API when turned on, stops it
    /// (socket and descriptor gone) when turned off. The "Debug API on" flag event follows from
    /// the core.
    ///
    /// No dialog is needed to turn it on, in any build: the bearer token is the
    /// gate. A script in the webview can flip the switch but cannot read the token file, so it
    /// gains nothing (and the token never crosses the bridge, see [`Host::debug_token_file`]).
    ///
    /// # Errors
    /// The I/O error when the setting cannot be saved (the switch is then not applied).
    pub async fn set_debug_api(&self, enabled: bool) -> std::io::Result<()> {
        let mut settings = HostSettings::load(&self.settings_path);
        settings.debug_api = Some(enabled);
        settings.save(&self.settings_path)?;
        let running = self.debug.lock().await.take();
        // Off or restarting: no window command runs as the Debug API under an earlier request's grant.
        self.ui.revoke_grants();
        if let Some(api) = running {
            if let Some(guard) = self.core.as_ref().and_then(Core::network_guard) {
                guard.remove_internal_port(api.port());
            }
            api.shutdown().await;
        }
        self.start_debug_api(Some(enabled)).await;
        Ok(())
    }

    /// The "keep Vibe Work sessions separate from the CLI" switch for Settings: the saved
    /// choice, whether the CLI home exists, and the CLI's version next to the bundled server's (the
    /// latter only while Vibe runs). Runs `vibe --version` (see [`crate::cli_sharing`]).
    pub async fn cli_sharing(
        &self,
        probe: &crate::cli_sharing::CliProbe,
    ) -> crate::cli_sharing::CliSharingView {
        let share = HostSettings::load(&self.settings_path).share_cli_sessions();
        let cli_version =
            crate::cli_sharing::detect_cli_version(probe, crate::cli_sharing::VERSION_TIMEOUT)
                .await;
        let bundled = self.core.as_ref().and_then(|c| c.status().vibe_version);
        let found = probe.cli_home.as_deref().is_some_and(Path::is_dir);
        crate::cli_sharing::CliSharingView::new(share, found, cli_version, bundled)
    }

    /// Saves the sharing switch. It applies the next time Vibe Work starts: the server's environment is
    /// fixed when it starts. Nothing else changes now.
    ///
    /// # Errors
    /// The I/O error when the setting cannot be saved.
    pub fn set_cli_sharing(&self, share: bool) -> std::io::Result<()> {
        let mut settings = HostSettings::load(&self.settings_path);
        settings.share_cli_sessions = Some(share);
        settings.save(&self.settings_path)
    }

    /// The Debug API token file's path (stable across runs; user-only), regenerating the token
    /// first when `rotate` is true: the old token stops working at once. Works while the API is
    /// off too (the next start reuses the new token).
    ///
    /// The result is a PATH. The token itself never leaves the Debug API crate except in the
    /// token file: not in this return value, not in any command result, event or `ui/*` reply
    /// (tests assert it with the real token value).
    ///
    /// # Errors
    /// [`vw_debug_api::DebugApiError`] when the new token cannot be written user-only (the old one
    /// keeps working then).
    pub async fn debug_token_file(
        &self,
        rotate: bool,
    ) -> Result<PathBuf, vw_debug_api::DebugApiError> {
        if let Some(api) = self.debug.lock().await.as_ref() {
            if rotate {
                api.rotate_token()?;
            }
            return Ok(api.token_path().to_path_buf());
        }
        if rotate {
            vw_debug_api::rotate_token_file(&self.descriptor_dir)
        } else {
            Ok(self.descriptor_dir.join(vw_debug_api::TOKEN_FILE))
        }
    }

    /// Stops the Debug API, the forwarder and the core (server shut down cleanly).
    pub async fn shutdown(&self) {
        self.forwarder.stop();
        self.events.stop();
        if let Some(api) = self.debug.lock().await.take() {
            api.shutdown().await;
        }
        if let Some(core) = &self.core {
            core.shutdown().await;
        }
        if let Some(keys) = &self.keys {
            keys.shutdown().await;
        }
    }
}
