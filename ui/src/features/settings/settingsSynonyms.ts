/**
 * The familiar words people type for each Settings row: the names other desktop apps, browsers, editors, operating
 * systems and coding agents use for the same setting, plus spelling variants. The catalog keeps each row's search
 * label equal to the words the row shows; this list is the row's second way in, so a person who searches with the
 * word they already know finds the row even where Vibe Work words it differently or uses its own product name.
 * Depends on nothing. Called by: catalog (merged into the search index), tests.
 * Invariants: keys are catalog ids (a test rejects an unknown key); each synonym is lower case and ranks its row
 * among the first results when typed on its own (tested); a row with no entry here is listed by a test for review.
 * Synonyms are never shown; they only find the row and rank it right after rows whose label matches.
 */
export const SETTING_SYNONYMS: Readonly<Record<string, readonly string[]>> = {
  // General
  "close-to-tray": ["minimize to tray", "close to tray", "run in background", "system tray"],
  "git-identity": ["git config", "user.name", "user.email", "commit author", "git identity"],
  "cli-sharing": ["vibe command line", "command line", "terminal", "vibe cli", "share chats"],
  "session-copies": ["worktree", "git worktrees", "cleanup worktrees"],
  "import-setup": [
    "import",
    "import settings",
    "migrate",
    "import from codex",
    "import from cursor",
    "bring your setup",
  ],
  "error-detail": ["verbose errors", "raw error", "error details"],
  "save-before-send": ["auto save", "autosave", "unsaved files"],
  "keyboard-shortcuts": ["shortcuts", "keys", "keybindings", "hotkeys", "command palette"],
  "gh-reads": ["github cli", "gh auth", "github token", "pull requests"],
  "keep-awake": ["prevent sleep", "stay awake", "caffeinate", "keep awake"],
  "send-to": ["context menu", "shell integration", "send to", "services menu", "ask vibe work"],
  connectors: ["integrations", "apps", "plugins", "connected apps"],
  "mcp-servers": ["mcp", "model context protocol", "tool servers"],
  // Appearance
  theme: ["dark mode", "light mode", "color theme", "colour theme", "night mode"],
  "open-files-in": ["editor tabs", "preview mode", "single tab"],
  "open-html-markdown-as": ["markdown preview", "html preview", "split view"],
  "mark-vibe-lines": ["highlight changes", "change markers", "gutter"],
  "status-usage": ["context usage", "token count", "status bar"],
  "show-thinking-expanded": ["chain of thought", "expand reasoning", "show reasoning"],
  // Accessibility
  "a11y-prompt-sound": ["audio cue", "chime", "notification sound", "accessibility signals"],
  "a11y-read-prompts": ["speak prompts", "read questions aloud"],
  "a11y-voice-answer": ["voice control", "answer by voice", "speak answers"],
  "a11y-reply-announce": ["screen reader", "announce replies", "voiceover", "narrator"],
  "a11y-text-size": ["font size", "zoom", "ui scale", "larger text", "bigger text"],
  "a11y-contrast": ["high contrast", "increase contrast"],
  "a11y-motion": ["reduce motion", "reduced motion", "animations"],
  // Account
  "account-facts": ["profile", "subscription", "signed in as"],
  "mistral-keys": ["switch account", "work key", "personal key", "multiple keys"],
  "mistral-key": ["api key", "mistral api key", "access token"],
  "vault-lock-on-quit": ["auto-lock", "lock on exit", "lock on quit", "auto lock"],
  "link-vibe-key": ["vibe api key", "create key"],
  "link-api-keys": ["mistral api keys", "revoke key", "rotate key"],
  "link-profile": ["change email", "account profile"],
  "link-security": ["2fa", "mfa", "two-factor", "change password"],
  "link-workspace": ["organization", "organisation", "switch workspace", "team"],
  // Project secrets
  "secret-scan": ["secret scanning", "leaked secrets", "redact", "leak scan"],
  "project-secrets": ["passwords", "secrets", "logins", "credentials", "environment variables"],
  "vault-mistral-key": ["mistral key in vault", "model key", "stored api key"],
  "add-entry": ["add vault entry", "add secret", "add password", "new secret", "add login"],
  // Usage and plan
  "plan-usage": ["quota", "usage limit", "credits left", "monthly usage"],
  "session-usage": ["token usage", "context window", "chat usage"],
  "link-monthly-usage": ["monthly quota", "usage this month"],
  "link-subscription": ["upgrade plan", "cancel subscription", "pricing plan"],
  "link-billing": ["invoices", "payment method", "spending limit"],
  "link-usage-dashboard": ["cost dashboard", "usage stats"],
  "link-rate-limits": ["rate limit", "requests per minute", "tokens per minute"],
  // Models and agent
  "default-model": ["model", "llm", "choose model", "ai model"],
  "default-thinking": ["reasoning effort", "thinking level", "effort", "extended thinking"],
  "default-access-mode": [
    "permission mode",
    "approval mode",
    "auto-approve",
    "yolo",
    "approval policy",
  ],
  "background-processes": ["background tasks", "dev server", "long-running commands"],
  subagents: ["sub-agents", "agents", "delegate", "parallel agents"],
  "stream-idle-timeout": ["timeout", "response timeout", "stream timeout", "idle timeout"],
  "compaction-point": ["auto compact", "auto-compact", "compact context", "summarize chat"],
  "image-generation": ["text to image", "image model", "generate images"],
  // Browser
  "parallel-browser-sessions": ["tab limit", "memory saver", "open tabs"],
  "browser-ad-blocking": [
    "ad blocker",
    "adblock",
    "tracker blocking",
    "content blocker",
    "shields",
  ],
  "browser-local-network-sites": ["local network access", "localhost access", "private network"],
  "builtin-control-enabled": ["built-in browser", "internal browser", "embedded browser"],
  "builtin-composer": ["show browser in composer", "built-in browser button"],
  "chrome-composer": ["chrome button in composer"],
  "edge-composer": ["edge button in composer"],
  "firefox-composer": ["firefox button in composer"],
  "safari-composer": ["safari button in composer"],
  "chrome-control": ["chrome extension", "browser extension", "google chrome", "connect chrome"],
  "chrome-control-enabled": ["turn on chrome extension", "chrome on"],
  "edge-control": ["edge extension", "microsoft edge", "connect edge"],
  "firefox-control": ["firefox extension", "firefox add-on", "connect firefox"],
  "safari-control": ["safari extension", "connect safari", "safari web extension"],
  "browser-secret-permissions": ["password autofill", "browser passwords", "login permissions"],
  // Safety
  "ask-allowed": ["permissions", "allowlist", "auto-approved", "what you have allowed"],
  "safety-delete-protected": ["rm -rf", "delete home folder", "dangerous commands"],
  "safety-permissions-protected": ["chmod", "chown", "recursive permissions"],
  "safety-disk-tools": ["format drive", "mkfs", "diskpart", "erase disk"],
  "safety-shutdown": ["shutdown", "reboot", "restart computer"],
  "safety-custom": [
    "your own safety rules",
    "custom rule",
    "blocklist",
    "deny list",
    "block command",
  ],
  // Allowed without asking
  "allowed-sessions": ["chat permissions", "allowed for this chat"],
  "allowed-earlier": ["always allow rules", "saved rules"],
  "allowed-vault": ["vault grants", "always answers", "secret grants"],
  // Instructions, Knowledge base
  instructions: ["agents.md", "custom instructions", "system prompt", "your instructions", "rules"],
  "knowledge-mistral": ["mistral libraries", "documents", "rag", "library"],
  "knowledge-local": ["local folders", "documents", "google drive", "onedrive", "dropbox"],
  // Voice
  microphone: ["input device", "audio input", "mic"],
  "dictation-language": [
    "speech to text language",
    "voice input language",
    "transcription language",
  ],
  "send-after-dictation": ["auto send", "send automatically"],
  narration: ["text to speech", "tts", "read aloud", "speak replies"],
  "narration-voice": ["tts voice", "speaker voice", "voice"],
  "voice-chat-auto-end": ["voice activity detection", "hands-free", "end of speech"],
  "voice-chat-pause": ["silence duration", "pause length"],
  // Notifications, Privacy and data
  "notification-level": ["notifications", "alerts", "desktop notifications", "notify"],
  "usage-data": ["telemetry", "analytics", "crash reports", "diagnostic data", "diagnostics"],
  "signed-in-accounts": ["cookies", "browser sign-ins", "sign out", "oauth", "logins"],
  experiments: ["beta features", "feature flags", "early access", "labs"],
  "link-vibe-training": ["training opt out", "use my data for training", "data training"],
  "link-api-training": ["api data training", "api training opt out"],
  "link-zdr": ["zdr", "no data retention"],
  "link-export": ["download my data", "data export", "gdpr"],
  "link-delete-account": ["close account", "remove account"],
  // Network
  "network-proxy": ["proxy", "proxy server", "http proxy", "corporate proxy", "company network"],
  "network-skip": ["no proxy", "no_proxy", "proxy bypass", "bypass list", "no proxy for"],
  "network-certificate": [
    "ca certificate",
    "root certificate",
    "ssl inspection",
    "custom ca",
    "certificates",
  ],
  // Updates, Developer, About
  "update-status": ["check now", "software update", "upgrade", "update now"],
  "auto-update": ["auto update", "automatic updates", "auto-update"],
  "developer-permissions": ["agent push", "git push", "developer access"],
  "debug-api": ["remote debugging", "automation api", "developer mode"],
  "debug-token": ["api token", "automation token"],
  "data-retention": ["storage", "where data is stored", "local data", "chat history"],
  about: ["about", "licence", "license", "about vibe work", "build number"],
};
