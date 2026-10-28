/**
 * Presentation model of the session list: which sessions show and how they are grouped. Pure
 * functions over `SessionSummary` and the user's project groups, so components only render.
 * Depends on: state/sessionSummary, state/projectGroups (types). Called by: SessionList, the palette, the `sessions`
 * UI route.
 * Invariants:
 * - Order everywhere is Vibe's own: `bumpedAt` (last real activity), then `updatedAt`, then id; projects keep the
 * user's order.
 * - Archived sessions show only while the archive filter is on.
 * - A child (subagent) session nests under its parent when the parent is in the shown set, and
 * stands on its own otherwise (a search that matches only the child must still show it).
 * - Filtering prunes the full list in place: a promoted child keeps its original project/shelf, rather than
 * being refiled according to its own pin/folder. Clearing restores the same rows and group order.
 * - A pinned top-level session appears only in the pinned shelf, never twice.
 * - "Needs you" lists every shown session with the `input` badge (children included) and is empty
 * while a search is active (results are not interrupted by a second group).
 * - Projects: a session shows in the project the core filed it into; "Not in a project" holds the
 * unfiled ones. A session the core has not filed yet shows where the core's folder rule will put it (the project
 * whose main folder is its folder, else a folder group), so nothing jumps when the filing arrives. Before the first
 * answer (`groups` null) the list groups by folder exactly as it did before projects existed.
 * - Empty projects show (they are drop targets) except while a search is active; a project whose name matches the
 * search shows all its sessions.
 * - Folder groups are keyed by the full `cwd`; two folders with the same last name get the parent folder added.
 * - Search is substring match over title, preview, folder paths and model, all words must match;
 * it is not full-text over transcripts.
 */
import type { GroupsData } from "../../state/projectGroups";
import { type SessionSummary } from "../../state/sessionSummary";

/** A session with its nested children. */
export interface RowNode {
  session: SessionSummary;
  children: readonly RowNode[];
  /** The session's folder name when it is not its project's main folder (shown as a chip), else `null`. */
  chip: string | null;
}

/** One group of the list: a project, or (before the core filed a session) its folder. */
export interface ProjectGroup {
  /** The project id; for a folder group its full `cwd` ("" for sessions without a folder). */
  key: string;
  /** The project id, `null` for a folder group. */
  id: string | null;
  /** Shown name: the project's name, or the folder's last name (with the parent when ambiguous). */
  name: string;
  /** The main folder (a folder group's folder), or `null` without one. */
  path: string | null;
  rows: readonly RowNode[];
  /** Any session in the group is running. */
  running: boolean;
  /** Sessions shown in the group, children included. */
  count: number;
  /** Sessions with an unread dot. */
  unread: number;
  /** Sessions the core files in the project (archived and not loaded ones included); `null` for a folder group. */
  filed: number | null;
}

/** The list as rendered. */
export interface ListModel {
  needs: readonly SessionSummary[];
  pinned: readonly RowNode[];
  projects: readonly ProjectGroup[];
  /** Sessions in no project ("Not in a project"); always empty before the groups arrived. */
  unfiled: readonly RowNode[];
  /** The project groups arrived (the list shows projects, not folders). */
  grouped: boolean;
  /** Sessions shown after the filters. */
  total: number;
  /** Archived sessions hidden by the archive filter (so the list can say so). */
  hiddenArchived: number;
}

/** Filters of the list. */
export interface ListOptions {
  query: string;
  showArchived: boolean;
}

function newestFirst(a: SessionSummary, b: SessionSummary): number {
  return b.bumpedAt - a.bumpedAt || b.updatedAt - a.updatedAt || a.id.localeCompare(b.id);
}

function haystack(s: SessionSummary): string {
  return [s.title, s.preview, s.cwd ?? "", ...s.extraFolders, s.model ?? ""]
    .join("\n")
    .toLowerCase();
}

function words(query: string): string[] {
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w !== "");
}

/** True when every word of `query` occurs in the session's searchable text. */
export function matchesQuery(s: SessionSummary, query: string): boolean {
  const all = words(query);
  if (all.length === 0) return true;
  const text = haystack(s);
  return all.every((w) => text.includes(w));
}

function segments(path: string): string[] {
  return path.split(/[\\/]+/).filter((p) => p !== "");
}

/** The folder form the core compares (trailing separators removed). */
function normalizeFolder(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed === "" || trimmed.endsWith(":") ? path : trimmed;
}

function label(path: string | null, withParent: boolean): string {
  if (path === null) return "No folder";
  const parts = segments(path);
  const last = parts[parts.length - 1] ?? path;
  const parent = parts[parts.length - 2];
  return withParent && parent !== undefined ? `${parent}/${last}` : last;
}

/** The last name of a folder (the chip text). */
export function folderName(path: string): string {
  return label(path, false);
}

function nest(
  sessions: readonly SessionSummary[],
  shown: ReadonlySet<string>,
  chipFor: (s: SessionSummary) => string | null,
): RowNode[] {
  const byParent = new Map<string, SessionSummary[]>();
  for (const s of sessions) {
    if (s.parentId !== null && shown.has(s.parentId)) {
      byParent.set(s.parentId, [...(byParent.get(s.parentId) ?? []), s]);
    }
  }
  const build = (s: SessionSummary, seen: ReadonlySet<string>): RowNode => ({
    session: s,
    // `seen` stops a corrupt parent cycle from recursing forever.
    children: seen.has(s.id)
      ? []
      : (byParent.get(s.id) ?? []).map((c) => build(c, new Set([...seen, s.id]))),
    chip: chipFor(s),
  });
  return sessions
    .filter((s) => s.parentId === null || !shown.has(s.parentId))
    .map((s) => build(s, new Set()));
}

function flatten(node: RowNode): SessionSummary[] {
  return [node.session, ...node.children.flatMap(flatten)];
}

/** `rows` again with each row's chip computed against `mainFolder` (`null`: every folder is a chip). */
function withChips(rows: readonly RowNode[], mainFolder: string | null): RowNode[] {
  const main = mainFolder === null ? null : normalizeFolder(mainFolder);
  const visit = (node: RowNode): RowNode => {
    const cwd = node.session.cwd;
    const chip =
      cwd === null || (main !== null && normalizeFolder(cwd) === main) ? null : folderName(cwd);
    return { ...node, chip, children: node.children.map(visit) };
  };
  return rows.map(visit);
}

function summary(
  key: string,
  id: string | null,
  name: string,
  path: string | null,
  rows: readonly RowNode[],
  filed: number | null,
): ProjectGroup {
  const everyone = rows.flatMap(flatten);
  return {
    key,
    id,
    name,
    path,
    rows,
    running: everyone.some((s) => s.badge === "run"),
    count: everyone.length,
    unread: everyone.filter((s) => s.unread).length,
    filed,
  };
}

/** Folder groups (before projects, or for sessions the core has not filed yet), in first-seen order. */
function folderGroups(nodes: readonly RowNode[], chips: boolean): ProjectGroup[] {
  const groups = new Map<string, RowNode[]>();
  for (const node of nodes) {
    const key = node.session.cwd ?? "";
    groups.set(key, [...(groups.get(key) ?? []), node]);
  }
  const bare = [...groups.keys()].map((k) => label(k === "" ? null : k, false));
  return [...groups.entries()].map(([key, rows]) => {
    const path = key === "" ? null : key;
    const ambiguous = bare.filter((b) => b === label(path, false)).length > 1;
    return summary(
      key,
      null,
      label(path, ambiguous),
      path,
      chips ? withChips(rows, path) : rows,
      null,
    );
  });
}

function shownSessions(
  all: readonly SessionSummary[],
  options: ListOptions,
  groups: GroupsData | null,
): SessionSummary[] {
  const query = options.query.trim();
  const terms = words(query);
  // A project whose name matches shows all its sessions (names are the user's, not in a session's text).
  const named = new Set(
    terms.length === 0 || groups === null
      ? []
      : groups.projects
          .filter((p) => terms.every((w) => p.name.toLowerCase().includes(w)))
          .map((p) => p.id),
  );
  return all
    .filter((s) => options.showArchived || !s.archived)
    .filter((s) => matchesQuery(s, query) || named.has(groups?.members[s.id] ?? ""))
    .sort(newestFirst);
}

/** Filters the canonical tree without changing a session's section or project while typing. */
function filteredList(full: ListModel, shown: readonly SessionSummary[]): ListModel {
  const ids = new Set(shown.map((s) => s.id));
  const prune = (rows: readonly RowNode[]): RowNode[] =>
    rows
      .flatMap((node) => {
        const children = prune(node.children);
        return ids.has(node.session.id) ? [{ ...node, children }] : children;
      })
      .sort((a, b) => newestFirst(a.session, b.session));
  return {
    ...full,
    needs: [],
    total: shown.length,
    pinned: prune(full.pinned),
    projects: full.projects
      .map((group) =>
        summary(group.key, group.id, group.name, group.path, prune(group.rows), group.filed),
      )
      .filter((group) => group.rows.length > 0),
    unfiled: prune(full.unfiled),
  };
}

/** Builds the list model from every known session, the filters and the project groups (`null`: not loaded yet). */
export function buildList(
  all: readonly SessionSummary[],
  options: ListOptions,
  groups: GroupsData | null = null,
): ListModel {
  const searching = options.query.trim() !== "";
  if (searching) {
    return filteredList(
      buildList(all, { ...options, query: "" }, groups),
      shownSessions(all, options, groups),
    );
  }
  const hiddenArchived = options.showArchived ? 0 : all.filter((s) => s.archived).length;
  const shown = shownSessions(all, options, groups);
  const ids = new Set(shown.map((s) => s.id));
  const roots = nest(shown, ids, () => null);
  const pinned = roots.filter((n) => n.session.pinned && !n.session.archived);
  const rest = roots.filter((n) => !pinned.includes(n));
  const base = {
    needs: shown.filter((s) => s.badge === "input"),
    pinned,
    total: shown.length,
    hiddenArchived,
  };
  if (groups === null) {
    return { ...base, projects: folderGroups(rest, false), unfiled: [], grouped: false };
  }
  const byId = new Map(groups.projects.map((p) => [p.id, p]));
  const byFolder = new Map<string, string>();
  for (const p of groups.projects) {
    if (p.mainFolder !== null && !byFolder.has(normalizeFolder(p.mainFolder))) {
      byFolder.set(normalizeFolder(p.mainFolder), p.id);
    }
  }
  const inProject = new Map<string, RowNode[]>();
  const pending: RowNode[] = [];
  const unfiled: RowNode[] = [];
  for (const node of rest) {
    const s = node.session;
    const member = groups.members[s.id];
    const predicted = s.cwd === null ? undefined : byFolder.get(normalizeFolder(s.cwd));
    const target =
      typeof member === "string" && byId.has(member)
        ? member
        : member === null
          ? null
          : (predicted ?? (s.cwd === null ? null : undefined));
    if (target === undefined) pending.push(node);
    else if (target === null) unfiled.push(node);
    else inProject.set(target, [...(inProject.get(target) ?? []), node]);
  }
  const projects = groups.projects.map((p) =>
    summary(
      p.id,
      p.id,
      p.name,
      p.mainFolder,
      withChips(inProject.get(p.id) ?? [], p.mainFolder),
      p.sessions,
    ),
  );
  return {
    ...base,
    projects: [...folderGroups(pending, true), ...projects],
    unfiled: withChips(unfiled, null),
    grouped: true,
  };
}

/** Names independently branched chats separately from owned subagent chats in a disclosure. */
export function childChatLabel(
  children: readonly RowNode[],
  owned: (id: string) => boolean,
): string {
  const agents = children.filter((child) => owned(child.session.id)).length;
  const branches = children.length - agents;
  const words = (count: number, kind: string) =>
    `${String(count)} ${kind} chat${count === 1 ? "" : "s"}`;
  return [
    branches > 0 ? words(branches, "branched") : "",
    agents > 0 ? words(agents, "subagent") : "",
  ]
    .filter(Boolean)
    .join(" and ");
}
