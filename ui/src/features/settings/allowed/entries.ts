/**
 * Merges the three lists of things Vibe may do without asking into one list of page entries:
 * the session answers the core keeps (`askGrants`), the earlier "Always allow" rules in Vibe's config
 * (`vibeRules list`) and the vault's lasting grants (`secrets list`: login fill grants, MCP key
 * grants). Each entry carries its row text (the card's covers words or the vault's own), its detail line, its project
 * (`null`: every project), its times, the plain text Find searches and the reference its revoke command needs.
 * Depends on: protocol/core types, covers, wording. Called by: allowed/view, allowed/store, tests.
 * Invariants: pure (no I/O, `now` passed in); entry keys are unique per kind and id; an MCP key grant whose word is
 * "none" allows nothing and is left out; earlier rules have no dates (Vibe applies them itself).
 */
import { sessionTitle as visibleTitle } from "../../../state/sessionSummary";
import { builtinTool, toolText } from "../../transcript/model/toolLabels";
import type {
  CoversPart,
  FillGrantView,
  RuleRef,
  ServerGrantView,
  SessionGrant,
  VibeRule,
} from "../../../protocol/core";
import { code, coversCapitalised, coversText, words } from "./covers";
import {
  folderName,
  relative,
  sessionAdded,
  sessionKind,
  sessionLast,
  shortDate,
  usage,
} from "./wording";

/** Which list an entry comes from; decides its revoke command and its status words. */
type EntryKind = "session" | "rule" | "fill" | "server";
/** The three groups of the page. */
export type GroupId = "sessions" | "earlier" | "vault";
/** The vault's sub-groups. */
export type VaultPart = "fills" | "servers";

/** One thing Vibe may do without asking, as the page shows it. */
export interface AllowedEntry {
  /** Unique over the page (`kind:id`). */
  key: string;
  kind: EntryKind;
  group: GroupId;
  /** The leaf list it is shown in: `earlier`, `sessions/{top session id}`, `vault/{part}`. */
  leaf: string;
  /** The row text, first letter upper case; code parts render as `<code>`. */
  what: CoversPart[];
  /** The `.d` line. */
  detail: string;
  /** The line in the bulk confirmation's list (plain, shown in mono). */
  line: string;
  /** Sort key for "Name" (plain row text). */
  name: string;
  /** What Find searches (lower case). */
  search: string;
  /** The project folder it applies in; `null` = every project. */
  project: string | null;
  addedAt: number | null;
  lastUsedAt: number | null;
  /** What its revoke command names: a grant id, or the rule. */
  ref: { id: string } | { rule: RuleRef };
}

/** A session as the page needs it. */
export interface SessionInfo {
  title: string;
  cwd: string | null;
  parentId: string | null;
}

/** The earlier rules list as read (or not). */
export interface RulesSource {
  state: "reading" | "ready" | "error";
  rules: readonly VibeRule[];
  /** Vibe's own default lists were read, so its shipped patterns are left out. */
  defaultsKnown: boolean;
  error: string | null;
}

/** The vault's lists as read (or not). */
export interface VaultSource {
  state: "reading" | "ready" | "locked" | "error";
  fills: readonly FillGrantView[];
  servers: readonly ServerGrantView[];
  error: string | null;
}

/** Everything the page is built from. */
export interface AllowedSources {
  grants: readonly SessionGrant[];
  sessions: Readonly<Record<string, SessionInfo>>;
  rules: RulesSource;
  vault: VaultSource;
}

/** The session a grant's row sits under: a subagent's answer goes under its parent session. */
export function topSession(id: string, sessions: Readonly<Record<string, SessionInfo>>): string {
  let at = id;
  // Bounded walk: a broken parent chain must not loop.
  for (let i = 0; i < 8; i++) {
    const parent = sessions[at]?.parentId ?? null;
    if (parent === null || parent === at) return at;
    at = parent;
  }
  return at;
}

/** The visible session name, including the shared fallback for a session no longer loaded. */
export function sessionTitle(id: string, sessions: Readonly<Record<string, SessionInfo>>): string {
  return visibleTitle({ title: sessions[id]?.title });
}

function haystack(...parts: (string | null | undefined)[]): string {
  return parts
    .filter((p): p is string => typeof p === "string" && p !== "")
    .join(" ")
    .toLowerCase();
}

/** A grant's covers words; a grant from a core without them gets words derived from its shape. */
function grantCovers(g: SessionGrant): CoversPart[] {
  if (g.covers.length > 0) return [...g.covers];
  if (g.everyCall)
    return [words("every call of "), words(toolText(g.tool, g.tool)), words(" in this session")];
  return [words(toolText(g.tool, g.labels.join(", ") || g.tool))];
}

function sessionEntry(g: SessionGrant, src: AllowedSources, now: number): AllowedEntry {
  const top = topSession(g.session, src.sessions);
  const what = coversCapitalised(grantCovers(g));
  const child = top === g.session ? null : sessionTitle(g.session, src.sessions);
  const detail = [
    sessionKind(g),
    `added ${sessionAdded(g.createdAt, now)}`,
    usage(g.uses, g.lastUsedAt, (ms) => sessionLast(ms, now)),
    ...(child === null ? [] : [`subagent “${child}”`]),
  ].join(" · ");
  const cwd = src.sessions[g.session]?.cwd ?? src.sessions[top]?.cwd ?? null;
  const text = coversText(what);
  return {
    key: `session:${g.id}`,
    kind: "session",
    group: "sessions",
    leaf: `sessions/${top}`,
    what,
    detail,
    line: text,
    name: text,
    search: haystack(text, detail, g.tool, sessionTitle(top, src.sessions), cwd && folderName(cwd)),
    project: cwd,
    addedAt: g.createdAt,
    lastUsedAt: g.lastUsedAt,
    ref: { id: g.id },
  };
}

/** An earlier rule's row words: "`bash`: commands that match `npm test *`", tool-wide "Every call of `bash`". */
export function ruleWhat(rule: VibeRule): CoversPart[] {
  const known = builtinTool(rule.tool);
  if (known !== null)
    return rule.pattern === null
      ? [words(`${known.name}: every call`)]
      : [words(`${known.name}: calls that match `), code(rule.pattern)];
  return rule.pattern === null
    ? [words("Every call of "), code(rule.tool)]
    : [code(rule.tool), words(": commands that match "), code(rule.pattern)];
}

function ruleEntry(rule: VibeRule): AllowedEntry {
  const what = ruleWhat(rule);
  const text = coversText(what);
  return {
    // Printable (it is also a focus target attribute); a tool name holds no "|".
    key: rule.pattern === null ? `rule:${rule.tool}` : `rule:${rule.tool}|${rule.pattern}`,
    kind: "rule",
    group: "earlier",
    leaf: "earlier",
    what,
    detail: "Every project",
    line: text,
    name: text,
    search: haystack(text, "every project"),
    project: null,
    addedAt: null,
    lastUsedAt: null,
    ref: {
      rule:
        rule.pattern === null ? { tool: rule.tool } : { tool: rule.tool, pattern: rule.pattern },
    },
  };
}

function fillEntry(g: FillGrantView, now: number): AllowedEntry {
  const detail = [
    `login “${g.loginName}”`,
    "every project",
    `added ${shortDate(g.grantedMs, now)}`,
    usage(g.uses, g.lastUsedMs, (ms) => relative(ms, now)),
  ].join(" · ");
  return {
    key: `fill:${g.id}`,
    kind: "fill",
    group: "vault",
    leaf: "vault/fills",
    what: [words(g.origin)],
    detail,
    line: g.origin,
    name: g.origin,
    search: haystack(g.origin, detail),
    project: null,
    addedAt: g.grantedMs,
    lastUsedAt: g.lastUsedMs,
    ref: { id: g.id },
  };
}

function serverEntry(g: ServerGrantView, now: number): AllowedEntry {
  const detail = [
    `key “${g.secretName}”`,
    g.grant === "always" ? "Always (until revoked)" : "Until Vibe Work closes",
    // A grant from before Vibe Work recorded its time has 0 here: no "added" then.
    ...(g.createdMs > 0 ? [`added ${shortDate(g.createdMs, now)}`] : []),
    usage(g.uses, g.lastUsedMs, (ms) => relative(ms, now)),
  ].join(" · ");
  return {
    key: `server:${g.id}`,
    kind: "server",
    group: "vault",
    leaf: "vault/servers",
    what: [words(g.server)],
    detail,
    line: g.server,
    name: g.server,
    search: haystack(g.server, detail, g.address),
    project: null,
    addedAt: g.createdMs > 0 ? g.createdMs : null,
    lastUsedAt: g.lastUsedMs,
    ref: { id: g.id },
  };
}

/** Every entry the sources hold (lists that are not read contribute nothing). */
export function allowedEntries(src: AllowedSources, now: number): AllowedEntry[] {
  return [
    ...src.grants.map((g) => sessionEntry(g, src, now)),
    ...(src.rules.state === "ready" ? src.rules.rules.map(ruleEntry) : []),
    ...(src.vault.state === "ready"
      ? [
          ...src.vault.fills.map((g) => fillEntry(g, now)),
          ...src.vault.servers.filter((g) => g.grant !== "none").map((g) => serverEntry(g, now)),
        ]
      : []),
  ];
}
