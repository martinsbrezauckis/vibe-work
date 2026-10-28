/**
 * The `/` menu: the GUI commands with what each one runs, and the session's skills from
 * `skills/list` as `/name` (sent as text; Vibe runs them server-side). Loaded with the menu
 * view (lazy chunk), never by the send path.
 * Depends on: model/slashNames. Called by: the slash menu view, the palette registration, tests.
 * Invariants: one row per `CLIENT_COMMANDS` name; commands come first, then skills (two sections). Within a section
 * rows are ranked: names that start with the query, then names that contain it, then descriptions with a word that starts with it
 * (table order inside a rank). Skills Vibe marks `userInvocable: false` or disabled are not offered.
 */
import type { JsonValue } from "../../../protocol/core";
import { TEXT } from "../../../vocabulary";
import { type ClientCommand, CLIENT_COMMANDS } from "./slashNames";

/** One GUI command row. */
export interface SlashCommandRow {
  name: ClientCommand;
  description: string;
  /** The underlying core command or protocol method, or "in the app" for client-only commands. Tooltips use the description. */
  runs: string;
}

/** The GUI commands, in menu order. */
export const SLASH_COMMANDS: readonly SlashCommandRow[] = [
  {
    name: "compact",
    description: "Summarize the history now (optional instructions)",
    runs: "session/compact",
  },
  {
    name: "clear",
    description: "Start a new chat",
    runs: "session/history/clear",
  },
  { name: "rename", description: "Rename this session", runs: "session/rename" },
  {
    name: "rewind",
    description: "Go back to your last message, restoring files",
    runs: "session/rewind",
  },
  {
    name: "branch",
    description: "Branch this session into a new session",
    runs: "session/fork",
  },
  {
    name: "retry",
    description: "Continue an interrupted reply (optional instructions)",
    runs: "turn/start",
  },
  { name: "model", description: TEXT.slashModel, runs: "config/model/write" },
  { name: "thinking", description: TEXT.slashThinking, runs: "config/model/write" },
  { name: "reload", description: "Reload config, skills and plugins", runs: "refreshPlugins" },
  {
    name: "status",
    description: "Model, context window, tokens and access mode",
    runs: "runtime/read",
  },
  { name: "log", description: "Where this session's log file is", runs: "session/log/read" },
  { name: "copy", description: "Copy the last answer", runs: "in the app" },
  { name: "help", description: "All commands and shortcuts", runs: "in the app" },
  // Vibe's plugin commands, then the commands that open the app's own page for them, then aliases.
  {
    name: "reload-plugins",
    description: "Reload this session's plugins and say what changed",
    runs: "refreshPlugins",
  },
  { name: "plugins", description: "The plugins this session runs", runs: "plugin/info" },
  {
    name: "loop",
    description: "Run a prompt on a timer: /loop 5m <prompt>",
    runs: "loops/create",
  },
  { name: "config", description: "Open Settings", runs: "in the app" },
  { name: "mcp", description: "Open Tools: connectors and MCP servers", runs: "in the app" },
  { name: "connectors", description: "Same as /mcp", runs: "in the app" },
  { name: "resume", description: "Show the sessions list", runs: "in the app" },
  { name: "continue", description: "Same as /resume", runs: "in the app" },
  { name: "todo", description: "Open the Plan tab with the todo list", runs: "in the app" },
  {
    name: "data-retention",
    description: "Where local data is kept and Mistral's data policy",
    runs: "in the app",
  },
  { name: "new", description: "Same as /clear", runs: "session/history/clear" },
];

/** A skill as the menu shows it. */
export interface SkillRow {
  name: string;
  description: string;
}

/** One menu item. */
export type MenuItem = (
  | { kind: "command"; row: SlashCommandRow }
  | { kind: "skill"; skill: SkillRow }
  | { kind: "manage" }
) & {
  /** Where the typed text sits inside the name (the letters the menu marks), or -1 when only the description matched. */
  hit: number;
};

/** The name of a menu item. */
export function itemName(item: MenuItem): string {
  return item.kind === "manage"
    ? "Manage skills…"
    : item.kind === "command"
      ? item.row.name
      : item.skill.name;
}

/** The one-line description of a menu item. */
export function itemDescription(item: MenuItem): string {
  return item.kind === "manage"
    ? "Open Skills"
    : item.kind === "command"
      ? item.row.description
      : item.skill.description;
}

/** The tooltip of a menu item: the action a pick takes, in words, and the keys that pick it. */
export function itemRuns(item: MenuItem): string {
  if (item.kind === "manage") return "Manage skills…";
  if (item.kind === "skill")
    return `Insert /${item.skill.name} skill into the message (Enter or Tab)`;
  return `/${item.row.name}: ${item.row.description} (Enter or Tab)`;
}

/** The skills of a `skills/list` reply worth offering (`SkillSummary`: invocable and enabled). */
export function skillRows(reply: JsonValue | undefined): SkillRow[] {
  const list =
    typeof reply === "object" && reply !== null && !Array.isArray(reply) ? reply["skills"] : null;
  if (!Array.isArray(list)) return [];
  const rows: SkillRow[] = [];
  for (const item of list) {
    if (typeof item !== "object" || item === null || Array.isArray(item)) continue;
    const name = item["name"];
    if (typeof name !== "string" || name === "") continue;
    if (item["userInvocable"] === false || item["enabled"] === false) continue;
    const description = item["description"];
    rows.push({ name, description: typeof description === "string" ? description : "" });
  }
  return rows;
}

/** The rank of a name/description against the query: 0 prefix, 1 inside the name, 2 a word of the description starts with it, -1 none. */
function rank(name: string, description: string, query: string): number {
  if (query === "") return 0;
  const n = name.toLowerCase();
  if (n.startsWith(query)) return 0;
  if (n.includes(query)) return 1;
  // The description counts from a word's start ("hist" finds "history"; "re" does not find "Where").
  const words = description.toLowerCase().split(/[^a-z0-9]+/);
  return words.some((w) => w.startsWith(query)) ? 2 : -1;
}

/** Keeps the rows that match, best rank first, table order inside a rank (a stable sort). */
function ranked<T extends { name: string; description: string }>(
  rows: readonly T[],
  query: string,
): { row: T; hit: number }[] {
  const out: { row: T; rank: number; hit: number }[] = [];
  for (const row of rows) {
    const r = rank(row.name, row.description, query);
    if (r >= 0)
      out.push({
        row,
        rank: r,
        hit: r === 2 || query === "" ? -1 : row.name.toLowerCase().indexOf(query),
      });
  }
  return out.sort((x, y) => x.rank - y.rank).map(({ row, hit }) => ({ row, hit }));
}

/**
 * The menu items for the typed text, or `null` when the menu should be closed (the text is not a
 * lone `/word` being typed). A word nothing matches gives an empty list (the menu says "No command
 * matches"). Case-insensitive; see the file header for the ranking.
 */
export function menuItems(text: string, skills: readonly SkillRow[]): MenuItem[] | null {
  const match = /^\/(\S*)$/.exec(text);
  if (match === null) return null;
  const query = (match[1] ?? "").toLowerCase();
  return [
    ...ranked(SLASH_COMMANDS, query).map(({ row, hit }): MenuItem => ({
      kind: "command",
      row,
      hit,
    })),
    ...ranked(skills, query).map(({ row, hit }): MenuItem => ({ kind: "skill", skill: row, hit })),
    ...("manage skills".includes(query) ? [{ kind: "manage" as const, hit: -1 }] : []),
  ];
}

/** Every GUI command has a row (used by tests and by the palette registration). */
export function tableIsComplete(): boolean {
  return CLIENT_COMMANDS.every((name) => SLASH_COMMANDS.some((row) => row.name === name));
}
