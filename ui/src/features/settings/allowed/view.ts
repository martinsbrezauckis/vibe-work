/**
 * What the Allowed without asking page shows for the entries and the person's choices:
 * Find over every entry (its words, detail line and project folder name, so typing a project's name filters by
 * project), the one fixed order, the groups and sub-groups with their counts, open/closed state, the
 * 10-rows-then-"Show 20 more" paging, the bulk links, the count line, the page state (reading, empty, no match) and
 * the Safety summary line. Pure.
 * Depends on: entries, wording. Called by: allowed/store (route reads), the page components, tests.
 * Invariants: groups start collapsed when the page holds more than 10 entries and open at 10 or fewer; while Find
 * filters, only groups and sub-groups with matches show, opened, and a bulk link names only the matches. Rows are
 * always in one order (`inPageOrder`): there is no Sort by and no Project selector.
 */
import {
  type AllowedEntry,
  type AllowedSources,
  type GroupId,
  sessionTitle,
  type VaultPart,
} from "./entries";
import { TEXT } from "../../../vocabulary";
import { plural } from "./wording";

/** The person's choices on the page (kept for the app run). */
export interface AllowedUi {
  query: string;
  /** Open or closed per group / sub-group key, as the person set it (no filter). */
  open: Readonly<Record<string, boolean>>;
  /** The same while a filter is on (reset when the filter changes; unset = open). */
  filterOpen: Readonly<Record<string, boolean>>;
  /** Rows shown per leaf list (unset = 10). */
  shown: Readonly<Record<string, number>>;
}

export const FIRST_ROWS = 10;
export const MORE_ROWS = 20;

/** The empty choices. */
export const INITIAL_UI: AllowedUi = {
  query: "",
  open: {},
  filterOpen: {},
  shown: {},
};

/** A list of rows: a group without sub-groups, or one sub-group. */
export interface LeafView {
  key: string;
  title: string;
  open: boolean;
  /** Matching entries (all of them, also the ones not shown yet). */
  entries: readonly AllowedEntry[];
  /** The rows shown now. */
  rows: readonly AllowedEntry[];
  notShown: number;
  /** "Revoke all 46…" / "Revoke these 17…" while open with entries; else null. */
  bulk: string | null;
  /** For a session sub-group: the session to open. */
  sessionId: string | null;
}

/** One group card. */
export interface GroupView {
  id: GroupId;
  title: string;
  help: string;
  count: number;
  open: boolean;
  /** The list could not be read: shown under the header whatever its open state. */
  problem: string | null;
  /** The vault is locked: its items are not shown or counted. */
  locked: boolean;
  /** Earlier rules: Vibe's own default list could not be read. */
  note: string | null;
  /** A group with its own rows (earlier rules). */
  leaf: LeafView | null;
  subgroups: readonly LeafView[];
}

/** The page. */
export interface PageView {
  state: "reading" | "empty" | "noMatch" | "list";
  filtering: boolean;
  total: number;
  matching: number;
  countLine: string;
  groups: readonly GroupView[];
}

const GROUP_TITLE: Record<GroupId, string> = {
  sessions: "Answers in open sessions",
  earlier: "Earlier “Always allow” rules",
  vault: TEXT.vault,
};

const GROUP_HELP: Record<GroupId, string> = {
  sessions: "“Allow for this session” answers.",
  earlier:
    "Saved by the “Always allow” button of Vibe Work 0.1.0 and 0.1.1. They apply in every project and every new session.",
  vault: "“Always …” answers from the vault's own questions.",
};

/** The vault's sub-groups, in page order. */
const VAULT_PARTS: readonly { part: VaultPart; title: string }[] = [
  { part: "fills", title: "Sites where a login may be filled" },
  { part: "servers", title: "MCP servers that get a key" },
];

const DEFAULTS_UNKNOWN =
  "Vibe Work could not read Vibe's own default list, so this list may also show rules that Vibe has by default.";
export const VAULT_LOCKED_PAGE = "The vault is locked, so its items are not shown or counted.";

/** Find is on. */
export function isFiltering(ui: AllowedUi): boolean {
  return ui.query.trim() !== "";
}

/** The entries Find keeps: every word is in the entry's Find text (which holds its project's folder name). */
function filterEntries(entries: readonly AllowedEntry[], ui: AllowedUi): AllowedEntry[] {
  const terms = ui.query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w !== "");
  return entries.filter((e) => terms.every((w) => e.search.includes(w)));
}

/** Newest first; a missing time after every present one; null when neither has one (the next key decides). */
function newer(x: number | null, y: number | null): number | null {
  if (x !== null && y !== null) return y - x || null;
  if (x !== null) return -1;
  if (y !== null) return 1;
  return null;
}

/**
 * The page's one order: last used first; entries never used after them, newest added first; entries without either
 * time (the earlier rules) by name.
 */
export function inPageOrder(entries: readonly AllowedEntry[]): AllowedEntry[] {
  return [...entries].sort(
    (a, b) =>
      newer(a.lastUsedAt, b.lastUsedAt) ??
      newer(a.addedAt, b.addedAt) ??
      (a.name.localeCompare(b.name) || a.key.localeCompare(b.key)),
  );
}

/** Open or closed for a group / sub-group key. */
function isOpen(key: string, ui: AllowedUi, filtering: boolean, total: number): boolean {
  if (filtering) return ui.filterOpen[key] ?? true;
  return ui.open[key] ?? total <= FIRST_ROWS;
}

function leaf(
  key: string,
  title: string,
  entries: readonly AllowedEntry[],
  ui: AllowedUi,
  filtering: boolean,
  total: number,
  sessionId: string | null = null,
): LeafView {
  const open = isOpen(key, ui, filtering, total);
  const rows = entries.slice(0, ui.shown[key] ?? FIRST_ROWS);
  return {
    key,
    title,
    open,
    entries,
    rows,
    notShown: entries.length - rows.length,
    bulk:
      open && entries.length > 0
        ? filtering
          ? `Revoke these ${String(entries.length)}…`
          : `Revoke all ${String(entries.length)}…`
        : null,
    sessionId,
  };
}

/** The "Show 20 more" label of a leaf ("Show 7 more (7 not shown)" when fewer are left). */
export function moreLabel(notShown: number): string {
  return `Show ${String(Math.min(MORE_ROWS, notShown))} more (${String(notShown)} not shown)`;
}

function countParts(src: AllowedSources, all: readonly AllowedEntry[]): string[] {
  const n = (group: GroupId) => all.filter((e) => e.group === group).length;
  return [
    `${plural(n("sessions"), "answer")} in open sessions`,
    src.rules.state === "error"
      ? "earlier rules could not be read"
      : plural(n("earlier"), "earlier rule"),
    src.vault.state === "locked"
      ? "the vault is locked, so its items are not counted"
      : src.vault.state === "error"
        ? "the vault could not be read"
        : `${String(n("vault"))} in the vault`,
  ];
}

/** Builds the page from the entries (all of them, unfiltered) and the person's choices. */
export function pageView(
  src: AllowedSources,
  all: readonly AllowedEntry[],
  ui: AllowedUi,
  keep: readonly GroupId[] = [],
): PageView {
  const filtering = isFiltering(ui);
  const reading = src.rules.state === "reading" || src.vault.state === "reading";
  const total = all.length;
  const matched = inPageOrder(filterEntries(all, ui));
  const of = (group: GroupId) => matched.filter((e) => e.group === group);
  const groups: GroupView[] = [];

  const sessions = of("sessions");
  const sessionIds = [...new Set(sessions.map((e) => e.leaf.slice("sessions/".length)))].sort(
    (a, b) => sessionTitle(a, src.sessions).localeCompare(sessionTitle(b, src.sessions)),
  );
  groups.push({
    ...base("sessions", sessions.length, ui, filtering, total),
    subgroups: sessionIds.map((id) =>
      leaf(
        `sessions/${id}`,
        `“${sessionTitle(id, src.sessions)}”`,
        sessions.filter((e) => e.leaf === `sessions/${id}`),
        ui,
        filtering,
        total,
        id,
      ),
    ),
  });

  const earlier = of("earlier");
  groups.push({
    ...base("earlier", earlier.length, ui, filtering, total),
    problem:
      src.rules.state === "error"
        ? `Could not read the earlier rules: ${src.rules.error ?? "unknown error"}. Nothing was changed. Try again, or restart Vibe Work if it keeps failing.`
        : null,
    note: src.rules.state === "ready" && !src.rules.defaultsKnown ? DEFAULTS_UNKNOWN : null,
    leaf: leaf("earlier", GROUP_TITLE.earlier, earlier, ui, filtering, total),
  });

  const vault = of("vault");
  groups.push({
    ...base("vault", vault.length, ui, filtering, total),
    problem:
      src.vault.state === "error"
        ? `Could not read the vault: ${src.vault.error ?? "unknown error"}. Nothing was changed. Try again, or restart Vibe Work if it keeps failing.`
        : null,
    locked: src.vault.state === "locked",
    subgroups: VAULT_PARTS.map(({ part, title }) =>
      leaf(
        `vault/${part}`,
        title,
        vault.filter((e) => e.leaf === `vault/${part}`),
        ui,
        filtering,
        total,
      ),
    ).filter((s) => s.entries.length > 0),
  });

  // Unfiltered, a group shows while it has entries, a problem, a locked vault or a status to say (`keep`).
  const shown = groups.filter((g) =>
    filtering ? g.count > 0 : g.count > 0 || g.problem !== null || g.locked || keep.includes(g.id),
  );
  const empty = total === 0 && shown.length === 0;
  return {
    state: reading
      ? "reading"
      : empty
        ? "empty"
        : filtering && matched.length === 0
          ? "noMatch"
          : "list",
    filtering,
    total,
    matching: matched.length,
    countLine: filtering
      ? `${String(matched.length)} of ${String(total)} match`
      : `${String(total)} allowed: ${countParts(src, all).join(" · ")}`,
    groups: shown,
  };
}

/** A group card before its own parts (rows, sub-groups, problem) are filled in. */
function base(
  id: GroupId,
  count: number,
  ui: AllowedUi,
  filtering: boolean,
  total: number,
): GroupView {
  return {
    id,
    title: GROUP_TITLE[id],
    help: GROUP_HELP[id],
    count,
    open: isOpen(id, ui, filtering, total),
    problem: null,
    locked: false,
    note: null,
    leaf: null,
    subgroups: [],
  };
}

/** The Safety summary line: its title, its `.d` line and whether Review shows. */
export interface SummaryView {
  state: "reading" | "none" | "some";
  title: string;
  detail: string;
  review: boolean;
}

/** The summary of everything allowed without asking, for Settings › Safety. */
export function summaryView(src: AllowedSources, all: readonly AllowedEntry[]): SummaryView {
  if (src.rules.state === "reading" || src.vault.state === "reading")
    return {
      state: "reading",
      title: "Allowed without asking",
      detail: "Reading what is allowed…",
      review: false,
    };
  const unreadable =
    src.rules.state === "error" || src.vault.state === "error" || src.vault.state === "locked";
  if (all.length === 0 && !unreadable)
    return {
      state: "none",
      title: "Nothing else is allowed without asking",
      detail: "",
      review: false,
    };
  const parts = countParts(src, all);
  // The Safety line names the earlier rules in full ("46 earlier “Always allow” rules").
  if (src.rules.state !== "error")
    parts[1] = plural(
      all.filter((e) => e.group === "earlier").length,
      "earlier “Always allow” rule",
    );
  return {
    state: "some",
    title: `You have allowed ${plural(all.length, "thing")} without asking`,
    detail: parts.join(" · "),
    review: true,
  };
}
