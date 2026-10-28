/**
 * What the Settings page holds: the categories in the order the column lists them, and an index of every
 * setting by name, description and keywords. The index feeds the search inside Settings ("In: Settings") and the
 * Settings results of the global search; each entry names the category that holds it and the `data-setting` anchor of
 * its row, so a hit opens the right category and lights up the right row.
 * Depends on: components/Icon (names only). Called by: SettingsNav, SettingsPage, the palette, settingsRoutes, tests.
 * Invariants: every entry's `category` is a listed category; every entry's id is unique and is the value of exactly
 * one `data-setting` attribute on the page (a test checks both), except the Tools page's items (`opens`); search is plain word matching (every typed word must
 * appear in the entry's text), so results are predictable, with label hits ranked before description hits.
 */
import type { IconName } from "../../components/Icon";
import { term, TEXT } from "../../vocabulary";
import { MISTRAL_LINKS } from "./mistralLinks";
import { chromeCatalog } from "./chrome/chromeRows";

/** A category of the Settings page. */
export interface SettingsCategory {
  id: string;
  label: string;
  icon: IconName;
}

/** The categories, in column order. */
export const SETTINGS_CATEGORIES: readonly SettingsCategory[] = [
  { id: "general", label: "General", icon: "sliders-horizontal" },
  { id: "appearance", label: "Appearance", icon: "sun-moon" },
  { id: "account", label: "Account", icon: "key-round" },
  { id: "secrets", label: "Project secrets", icon: "lock" },
  { id: "usage", label: "Usage and plan", icon: "gauge" },
  { id: "models", label: "Models and agent", icon: "cpu" },
  { id: "browser", label: "Browser", icon: "globe" },
  { id: "safety", label: "Safety", icon: "triangle-alert" },
  // All permissions live together directly under Safety.
  { id: "allowed", label: "Allowed without asking", icon: "list-checks" },
  { id: "instructions", label: "Instructions", icon: "scroll-text" },
  { id: "voice", label: "Voice", icon: "audio-lines" },
  { id: "notifications", label: "Notifications", icon: "bell" },
  { id: "privacy", label: "Privacy and data", icon: "shield" },
  { id: "network", label: "Network", icon: "server" },
  { id: "updates", label: "Updates", icon: "refresh-cw" },
  { id: "developer", label: "Developer", icon: "code" },
  { id: "about", label: "About", icon: "info" },
];

/** One setting in the index. */
export interface SettingEntry {
  /** The `data-setting` anchor of its row on the page. */
  id: string;
  category: string;
  label: string;
  description: string;
  /** Extra words people may type (not shown). */
  keywords: string;
  /** Lives on another page (Connectors): a hit opens that page instead of a Settings category. */
  opens?: "connectors";
}

/** Every setting, in page order. */
export const SETTINGS_INDEX: readonly SettingEntry[] = [
  {
    id: "ask-allowed",
    category: "safety",
    label: "What you have allowed without asking",
    description:
      "How many answers, earlier rules and vault grants let Vibe act without asking; Review lists them.",
    keywords: "allowed without asking always grants revoke approved",
  },
  {
    id: "allowed-sessions",
    category: "allowed",
    label: "Answers in open sessions",
    description:
      "“Allow for this session” answers, per open session; revoke one to be asked again.",
    keywords: "always allow earlier rules vault grant command site mcp key revoke session answers",
  },
  {
    id: "allowed-earlier",
    category: "allowed",
    label: "Earlier “Always allow” rules",
    description:
      "Rules saved by the “Always allow” button of Vibe Work 0.1.0 and 0.1.1; revoke them here.",
    keywords: "always allow earlier rules vault grant command site mcp key revoke",
  },
  {
    id: "allowed-vault",
    category: "allowed",
    label: "Vault: lasting “Always” answers",
    description:
      "Commands that may use a secret, sites where a login may be filled, MCP servers that get a key.",
    keywords: "always allow earlier rules vault grant command site mcp key revoke secret login",
  },
  {
    id: "git-identity",
    category: "general",
    label: "Git name and email",
    description: "The author identity for commits and new repositories in Vibe Work.",
    keywords: "git commit author identity name email repository version history",
  },
  {
    id: "cli-sharing",
    category: "general",
    label: "Keep sessions separate from the Vibe command line",
    description:
      "Whether sessions started here also show in the CLI's list, and the other way round.",
    keywords: "cli terminal command line share sharing vibe home",
  },
  {
    id: "session-copies",
    category: "general",
    label: "Session copies",
    description: "Review or remove session copies (git worktrees) no chat uses anymore.",
    keywords: "git worktree worktrees leftover kept copies review remove cleanup",
  },
  {
    id: "error-detail",
    category: "general",
    label: "Show the provider's own error text",
    description: "Adds Mistral's own error message under the plain explanation.",
    keywords: "errors details provider message raw debug",
  },
  {
    id: "save-before-send",
    category: "general",
    label: "Save open files before Vibe works",
    description:
      "When you send a message, edited files open in the editor are saved first, so the file on disk is what you see.",
    keywords: "editor save unsaved files send message disk before",
  },
  {
    id: "gh-reads",
    category: "general",
    label: "Let Vibe read GitHub with your gh login",
    description:
      "Read pull requests, issues, runs and the GitHub API with your gh login. Vibe never sees your token and cannot write.",
    keywords: "github gh pull request issue ci run api token read",
  },
  {
    id: "keep-awake",
    category: "general",
    label: "Keep this computer awake while Vibe is working",
    description: "While a chat is working, your computer does not go to sleep on its own.",
    keywords: "sleep awake power idle battery screen lock lid",
  },
  {
    id: "send-to",
    category: "general",
    label: "Right-click menus",
    description:
      'Adds "Send to Vibe Work" to the right-click menus of your file manager (and on a Mac "Ask Vibe Work" for selected text).',
    keywords:
      "send to vibe work right click context menu explorer finder file manager files folders services selected text ask",
  },
  {
    id: "theme",
    category: "appearance",
    label: "Theme",
    description: "Dark or light.",
    keywords: "dark light mode colour color appearance",
  },
  {
    id: "open-files-in",
    category: "appearance",
    label: "Open files in",
    description: "Open files in separate tabs or one File tab with a picker.",
    keywords: "editor files tabs separate single picker",
  },
  {
    id: "open-html-markdown-as",
    category: "appearance",
    label: "Open HTML and Markdown as",
    description: "Preview, code, or code with its preview beside it.",
    keywords: "editor preview markdown html code side split render",
  },
  {
    id: "mark-vibe-lines",
    category: "appearance",
    label: "Mark lines Vibe changes in open files",
    description:
      "The editor marks the lines Vibe changed while the file was open, until you press Got it.",
    keywords: "editor highlight changed lines agent orange marks",
  },
  {
    id: "status-usage",
    category: "appearance",
    label: "Show usage in the status bar",
    description: 'How full the open session\'s context is, as "Context 42%".',
    keywords: "usage context tokens footer status bar compaction",
  },
  {
    id: "show-thinking-expanded",
    category: "appearance",
    label: "Show thinking expanded",
    description: "Open thinking blocks in the session by default.",
    keywords: "transcript composer thinking reasoning expand collapse disclosure",
  },
  {
    id: "account-facts",
    category: "account",
    label: "Your account",
    description: "Plan and key type in plain words, name and Workspace.",
    keywords: "plan pro subscription key type workspace organization name",
  },
  {
    id: "mistral-keys",
    category: "account",
    label: "Mistral keys",
    description: "Keep a personal and a work key and switch between them.",
    keywords: "account workspace switch key personal work organization",
  },
  {
    id: "mistral-key",
    category: "account",
    label: "Mistral key",
    description: "Test, replace or lock the key.",
    keywords: "api key vault password lock unlock token",
  },
  {
    id: "vault-lock-on-quit",
    category: "account",
    label: "Lock the vault when Vibe Work quits",
    description:
      "When off, an unlocked vault stays unlocked after a restart: its key is kept in this computer's credential store.",
    keywords:
      "vault lock quit exit restart start password keychain credential store remember forget shared computer",
  },
  {
    id: "secret-scan",
    category: "secrets",
    label: "Scan session logs for my secrets",
    description: "Find Vault secrets that ended up in chat logs, and redact them",
    keywords: "scan leak leaked log logs session redact remove secret token key find",
  },
  {
    id: "project-secrets",
    category: "secrets",
    label: "Vault entries",
    description:
      "Search secrets, website logins and MCP keys; manage project permissions and grants.",
    keywords:
      "secret secrets api key token vault deploy env environment variable broker login password website mcp grant revoke permission allowed ask never",
  },
  {
    id: "vault-mistral-key",
    category: "secrets",
    label: "Mistral key in Vault",
    description: "See the stored model key's source and test result; manage it in Account.",
    keywords: "model api key vault test source account",
  },
  {
    id: "add-entry",
    category: "secrets",
    label: "Add Vault entry",
    description:
      "Store a secret, website login or MCP key; choose a project to set its permission.",
    keywords: "add new secret login password mcp token store vault generate",
  },
  {
    id: "safety-delete-protected",
    category: "safety",
    label: "Stop deleting your home folder, drives or the project's parent",
    description: "A safety reminder for agent commands like rm -rf ~ or Remove-Item -Recurse C:\\.",
    keywords: "safety rm rf delete home root drive dangerous reminder hook",
  },
  {
    id: "safety-permissions-protected",
    category: "safety",
    label: "Stop recursive permission changes on your home or system folders",
    description: "chmod -R or chown -R on your home folder or a system root.",
    keywords: "safety chmod chown permissions owner recursive",
  },
  {
    id: "safety-disk-tools",
    category: "safety",
    label: "Stop disk tools",
    description: "mkfs, dd to a device, diskpart, format: they can erase a drive.",
    keywords: "safety disk format mkfs dd diskpart erase",
  },
  {
    id: "safety-shutdown",
    category: "safety",
    label: "Stop shutdown and restart",
    description: "An agent never shuts down or restarts the computer.",
    keywords: "safety shutdown reboot restart poweroff",
  },
  {
    id: "safety-custom",
    category: "safety",
    label: "Your own safety rules",
    description: "Stop any agent command that contains a text you choose.",
    keywords: "safety custom rule block command pattern",
  },
  {
    id: "default-model",
    category: "models",
    label: TEXT.defaultModel,
    description: TEXT.defaultModelDescription,
    keywords: "model medium devstral magistral reasoning",
  },
  {
    id: "default-thinking",
    category: "models",
    label: TEXT.defaultThinkingLevel,
    description: TEXT.defaultThinkingDescription,
    keywords: "thinking reasoning effort level off high default",
  },
  {
    id: "default-access-mode",
    category: "models",
    label: TEXT.accessMode,
    description: TEXT.accessDefaultDescription,
    keywords: "access mode default agent profile permissions ask auto approve full access",
  },
  {
    id: "background-processes",
    category: "models",
    label: "Background processes",
    description: "Whether the agent may start commands that keep running.",
    keywords: "background process dev server long running",
  },
  {
    id: "subagents",
    category: "models",
    label: "Subagents",
    description: "Whether the agent may hand parts of a task to helper agents.",
    keywords: "subagent helper agents delegate",
  },
  {
    id: "parallel-browser-sessions",
    category: "browser",
    label: "Live browser tabs",
    description:
      "How many page tabs can keep a browser engine open; background tabs pause at the limit. Auto adjusts to available memory.",
    keywords: "browser tabs pool parallel memory capacity auto",
  },
  ...chromeCatalog(),
  {
    id: "browser-close-user-tabs",
    category: "browser",
    label: term("browserPermissions", "closeUserTabs").name,
    description: term("browserPermissions", "closeUserTabs").description,
    keywords:
      "browser agent close user tabs opened delegation permission keep ask me default allow",
  },
  {
    id: "browser-secret-permissions",
    category: "browser",
    label: TEXT.secretPermissionsTitle,
    description: TEXT.secretPermissionsSummary,
    keywords: "vault secret delegation access permissions entries project open",
  },
  {
    id: "stream-idle-timeout",
    category: "models",
    label: "A silent answer counts as stalled after",
    description: "How long Vibe waits in a silent answer before it retries.",
    keywords: "timeout stall stream idle network retry",
  },
  {
    id: "image-generation",
    category: "models",
    label: "Image generation",
    description:
      "Image generation with Mistral; asks like any other tool, by the session's access mode.",
    keywords: "image images picture generate flux draw art cost",
  },
  {
    id: "session-usage",
    category: "usage",
    label: TEXT.thisSession,
    description: "Tokens used and how full the context is, until Vibe compacts the session.",
    keywords: "tokens context usage compaction session",
  },
  {
    id: "instructions",
    category: "instructions",
    label: "Your instructions",
    description: "Your personal AGENTS.md: plain text Vibe reads before it works, for every chat.",
    keywords: "agents.md agents md prompt rules custom system personal",
  },
  {
    id: "microphone",
    category: "voice",
    label: "Microphone",
    description: "The input used for dictation.",
    keywords: "mic input audio dictation speech",
  },
  {
    id: "dictation-language",
    category: "voice",
    label: "Dictation language",
    description: "The language you speak when you dictate.",
    keywords: "speech language transcription voxtral",
  },
  {
    id: "send-after-dictation",
    category: "voice",
    label: "Send after dictation",
    description: "Send the message as soon as the dictated text is in the box.",
    keywords: "autosend auto send dictation voice speech",
  },
  {
    id: "narration",
    category: "voice",
    label: "Read finished turns aloud",
    description: "Narration: Vibe Work reads a short summary of each finished turn.",
    keywords: "narration read aloud speak tts speech voice",
  },
  {
    id: "narration-voice",
    category: "voice",
    label: "Narration voice",
    description: "The voice that reads turns aloud.",
    keywords: "narration tts speaker voice",
  },
  {
    id: "voice-chat-auto-end",
    category: "voice",
    label: "End my turn after a pause",
    description: "Voice chat sends what you said when you stop talking.",
    keywords: "voice chat conversation talk pause turn silence automatic",
  },
  {
    id: "voice-chat-pause",
    category: "voice",
    label: "Pause before sending",
    description: "How long voice chat waits after you stop talking.",
    keywords: "voice chat pause silence seconds turn",
  },
  {
    id: "notification-level",
    category: "notifications",
    label: "Notification level",
    description: "Errors only, plus permission requests, or everything.",
    keywords: "notifications alerts desktop notify system tell me about",
  },
  {
    id: "usage-data",
    category: "privacy",
    label: "Send usage data to Mistral",
    description: "Anonymous product usage from Vibe: which features are used, errors, timings.",
    keywords: "telemetry analytics usage data privacy",
  },
  {
    id: "signed-in-accounts",
    category: "privacy",
    label: "Signed-in accounts",
    description: "Sign in once in the browser, or clear a provider's saved session.",
    keywords:
      "google github microsoft apple sign in oauth browser cookies session provider login consent",
  },
  {
    id: "experiments",
    category: "privacy",
    label: "Take part in Mistral's experiments",
    description:
      "Mistral switches features on for groups of accounts. It needs usage data to be on.",
    keywords: "experiments features rollout privacy",
  },
  {
    id: "network-proxy",
    category: "network",
    label: "Proxy for company networks",
    description:
      "Send Vibe Work's and Vibe's internet requests through your company's proxy; test the connection.",
    keywords:
      "proxy http https company corporate network firewall vpn address port sign in connection test offline cannot reach mistral",
  },
  {
    id: "network-skip",
    category: "network",
    label: "Addresses that skip the proxy",
    description: "Hosts, domains and ranges that are reached directly, not through the proxy.",
    keywords: "no proxy no_proxy bypass skip exclude intranet direct localhost domain range cidr",
  },
  {
    id: "network-certificate",
    category: "network",
    label: "Company certificate",
    description:
      "Trust your company's certificate file when its proxy checks web traffic with its own certificate.",
    keywords:
      "certificate ssl tls ca pem authority self-signed inspection trust cert ssl_cert_file untrusted",
  },
  {
    id: "update-status",
    category: "updates",
    label: "Check for updates",
    description: "The installed version, a newer one if there is one, and Check now.",
    keywords: "update version new release check download install",
  },
  {
    id: "auto-update",
    category: "updates",
    label: "Download updates automatically",
    description: "A new version downloads in the background; nothing installs without your click.",
    keywords: "update automatic background download",
  },
  {
    id: "debug-api",
    category: "developer",
    label: "Debug API",
    description:
      "Lets an agent or script on this computer drive and test Vibe Work, on 127.0.0.1 only.",
    keywords: "debug api agent automation script localhost developer",
  },
  {
    id: "debug-token",
    category: "developer",
    label: "Debug API token file",
    description: "Agents read the token from this file; regenerate it to lock old ones out.",
    keywords: "debug token secret regenerate file",
  },
  {
    id: "data-retention",
    category: "about",
    label: "Data retention",
    description: "Where Vibe Work keeps local data and Mistral's data policy.",
    keywords: "privacy storage history sessions logs captures vault files folders retention policy",
  },
  {
    id: "about",
    category: "about",
    label: "Version and licence",
    description:
      "Vibe Work's version. Independent project, not affiliated with Mistral AI. MIT License.",
    keywords:
      "about version licence license mit independent author source code github website homepage shellx",
  },
];

/** Connectors and MCP servers have their own page; Settings search still finds them. */
const ELSEWHERE: readonly SettingEntry[] = [
  {
    id: "connectors",
    category: "general",
    label: "Connectors",
    description: "Sign in to apps Vibe can use (GitHub, Notion, Gmail…): on the Tools page.",
    keywords: "connectors apps integrations sign in tools github slack",
    opens: "connectors",
  },
  {
    id: "mcp-servers",
    category: "general",
    label: "MCP servers",
    description: "Add, sign in to, turn off or remove MCP servers: on the Tools page.",
    keywords: "mcp servers tools add custom",
    opens: "connectors",
  },
];

/** Everything Settings search covers: the settings, the links to Mistral's pages, and the Tools page's items. */
export const SEARCH_INDEX: readonly SettingEntry[] = [
  ...SETTINGS_INDEX,
  ...MISTRAL_LINKS.map((l) => ({
    id: l.id,
    category: l.category,
    label: l.label,
    description: l.note,
    keywords: `${l.keywords} mistral website link`,
  })),
  ...ELSEWHERE,
];

/** The category with `id`, else the first one. */
export function categoryOf(id: string): SettingsCategory {
  return (
    SETTINGS_CATEGORIES.find((c) => c.id === id) ?? (SETTINGS_CATEGORIES[0] as SettingsCategory)
  );
}

/** The words of a query (lower case, empty words dropped). */
function queryWords(query: string): string[] {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w !== "");
}

/**
 * Settings matching `query`: every word must appear in the label, description, keywords or category name. Label hits
 * first, then the rest, each in page order. An empty query matches nothing.
 */
export function searchSettings(query: string, chromeAvailable = true): SettingEntry[] {
  const words = queryWords(query);
  if (words.length === 0) return [];
  const hits = SEARCH_INDEX.filter((e) => {
    if (!chromeAvailable && e.id.startsWith("chrome-control")) return false;
    const hay =
      `${e.label} ${e.description} ${e.keywords} ${categoryOf(e.category).label}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  const inLabel = (e: SettingEntry) => words.some((w) => e.label.toLowerCase().includes(w));
  return [...hits.filter(inLabel), ...hits.filter((e) => !inLabel(e))];
}

/** The path a result shows: `Settings › Voice`, or `Tools` for the Tools page's items. */
export function settingPath(entry: SettingEntry): string[] {
  return entry.opens === "connectors" ? ["Tools"] : ["Settings", categoryOf(entry.category).label];
}

/**
 * Splits `text` into plain and matched runs for highlighting (case-insensitive, every query word). Adjacent and
 * overlapping matches merge.
 */
export function highlightRuns(text: string, query: string): { text: string; hit: boolean }[] {
  const words = queryWords(query);
  const lower = text.toLowerCase();
  const marks: boolean[] = new Array<boolean>(text.length).fill(false);
  for (const w of words) {
    let at = lower.indexOf(w);
    while (at !== -1) {
      for (let i = at; i < at + w.length; i++) marks[i] = true;
      at = lower.indexOf(w, at + w.length);
    }
  }
  const runs: { text: string; hit: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const hit = marks[i] === true;
    const last = runs[runs.length - 1];
    if (last !== undefined && last.hit === hit) last.text += text.charAt(i);
    else runs.push({ text: text.charAt(i), hit });
  }
  return runs;
}
