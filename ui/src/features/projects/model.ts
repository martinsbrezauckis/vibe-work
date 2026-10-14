/**
 * The Projects overview's presentation model:
 * a project's status (from its manifest), its sessions as chips, the "Needs you" rows across projects, the card order
 * (most recent activity first), the status filter and its counts, plan progress and the health line from the
 * validator's findings. Pure functions, so the components only render.
 * Depends on: protocol/core types, state/sessionSummary, state/projects (filter type), pages/model (progress).
 * Called by: the Projects page components, the `ui/projects` route, UI tests.
 * the overview lists the user's projects (`OverviewItem` = a project and its main folder's card, `null`
 * without a main folder or before the card arrived); a project's sessions are the ones filed into it.
 * Invariants: page text is passed through as plain text (never markup); a project without a manifest status counts
 * as active; archived projects show only under the Archived filter; before the groups arrive the items are the
 * folder cards (as before projects existed).
 */
import type { ProjectCard, ProjectHub, ProjectView } from "../../protocol/core";
import type { GroupsData } from "../../state/projectGroups";
import type { SessionSummary } from "../../state/sessionSummary";
import type { StatusFilter } from "../../state/projects";
import { planProgress } from "../pages/model";

/** Project statuses in display order (Handbook Format 1.1 §3). */
export const STATUS_ORDER = ["active", "maintenance", "paused", "archived"] as const;
/** One project status. */
export type ProjectStatus = (typeof STATUS_ORDER)[number];
/** Labels of the statuses. */
export const STATUS_LABEL: Record<ProjectStatus, string> = {
  active: "Active",
  maintenance: "Maintenance",
  paused: "Paused",
  archived: "Archived",
};

/** One project of the overview: the user's project and its main folder's handbook card. */
export interface OverviewItem {
  project: ProjectView;
  card: ProjectCard | null;
}

/** The overview's items: the projects in the user's order with their cards (`groups` null: the folder cards). */
export function overviewItems(
  cards: readonly ProjectCard[],
  groups: GroupsData | null,
): OverviewItem[] {
  if (groups === null) {
    return cards.map((card) => ({
      project: {
        id: card.project,
        name: displayName(card),
        mainFolder: card.project,
        folderName: card.name,
        auto: true,
        sessions: 0,
      },
      card,
    }));
  }
  return groups.projects.map((project) => ({
    project,
    card:
      project.mainFolder === null
        ? null
        : (cards.find((c) => c.project === project.mainFolder) ?? null),
  }));
}

/** The item's status: its card's (see `projectStatus`), active without a card. */
export function itemStatus(item: OverviewItem): ProjectStatus {
  return item.card === null ? "active" : projectStatus(item.card);
}

/** The project's status: the manifest's when it is one of §3, else active. */
export function projectStatus(card: ProjectCard): ProjectStatus {
  const s = card.manifest?.status ?? "active";
  return (STATUS_ORDER as readonly string[]).includes(s) ? (s as ProjectStatus) : "active";
}

/** A session on a card. */
export interface CardSession {
  id: string;
  title: string;
  /** `working` (running), `waiting` (needs the user), `idle`. */
  state: "working" | "waiting" | "idle";
  bumpedAt: number;
}

/** The open top-level sessions of `item`'s project (filed into it; before the groups: working in its folder). */
export function sessionsOf(
  item: OverviewItem,
  sessions: readonly SessionSummary[],
  members: GroupsData["members"] | null,
): CardSession[] {
  const inProject = (s: SessionSummary) =>
    members === null ? s.cwd === item.project.mainFolder : members[s.id] === item.project.id;
  return sessions
    .filter((s) => inProject(s) && !s.archived && s.parentId === null)
    .sort((a, b) => b.bumpedAt - a.bumpedAt)
    .map((s) => ({
      id: s.id,
      title: s.title,
      state: s.badge === "run" ? "working" : s.badge === "input" ? "waiting" : "idle",
      bumpedAt: s.bumpedAt,
    }));
}

/** A "Needs you" row. */
export interface NeedsItem {
  sessionId: string;
  title: string;
  /** The project's shown name. */
  where: string;
  bumpedAt: number;
}

/** Sessions across all projects that wait for the user (an approval or a question), newest first. */
export function needsYou(
  sessions: readonly SessionSummary[],
  items: readonly OverviewItem[],
  members: GroupsData["members"] | null,
): NeedsItem[] {
  return sessions
    .filter((s) => s.badge === "input" && !s.archived)
    .sort((a, b) => b.bumpedAt - a.bumpedAt)
    .map((s) => {
      const item = items.find((i) =>
        members === null ? i.project.mainFolder === s.cwd : members[s.id] === i.project.id,
      );
      return {
        sessionId: s.id,
        title: s.title,
        where: item === undefined ? s.project : item.project.name,
        bumpedAt: s.bumpedAt,
      };
    });
}

/** The name a card shows: the manifest's project name, else the folder name. */
export function displayName(card: ProjectCard): string {
  return card.manifest?.project ?? card.name;
}

/** Last activity of a project: its handbook files or its newest session, ms since the epoch. */
export function activity(
  item: OverviewItem,
  sessions: readonly SessionSummary[],
  members: GroupsData["members"] | null,
): number {
  const newest = Math.max(0, ...sessionsOf(item, sessions, members).map((s) => s.bumpedAt));
  return Math.max(item.card?.changedMs ?? 0, newest);
}

/** Items matching the filter and the search text, grouped by status in display order, each group newest first. */
export function groupItems(
  items: readonly OverviewItem[],
  sessions: readonly SessionSummary[],
  members: GroupsData["members"] | null,
  filter: StatusFilter,
  query: string,
): Array<{ status: ProjectStatus; items: OverviewItem[] }> {
  const q = query.trim().toLowerCase();
  const visible = items.filter((i) => {
    const status = itemStatus(i);
    const shown = filter === "all" ? status !== "archived" : status === filter;
    const card = i.card;
    const text = [
      i.project.name,
      card === null ? "" : displayName(card),
      card?.manifest?.description ?? "",
      i.project.mainFolder ?? "",
    ]
      .join(" ")
      .toLowerCase();
    return shown && (q === "" || text.includes(q));
  });
  const recent = (i: OverviewItem) => activity(i, sessions, members);
  return STATUS_ORDER.map((status) => ({
    status,
    items: visible
      .filter((i) => itemStatus(i) === status)
      .sort((a, b) => recent(b) - recent(a) || a.project.name.localeCompare(b.project.name)),
  })).filter((g) => g.items.length > 0);
}

/** Counts per filter button (`all` leaves archived projects out). */
export function filterCounts(items: readonly OverviewItem[]): Record<StatusFilter, number> {
  const count = (s: ProjectStatus) => items.filter((i) => itemStatus(i) === s).length;
  return {
    all: items.length - count("archived"),
    active: count("active"),
    maintenance: count("maintenance"),
    paused: count("paused"),
    archived: count("archived"),
  };
}

/** A plan's progress as the card shows it. */
export interface PlanLine {
  title: string;
  done: number;
  total: number;
  /** The step being done now, if any. */
  now: string | null;
  /** Width of the done part and the doing part of the bar, percent. */
  donePct: number;
  nowPct: number;
}

/** The progress line of a plan summary. */
export function planLine(plan: ProjectCard["plans"][number]): PlanLine {
  const p = planProgress(plan.steps);
  const now = p.doing >= 0 ? (plan.steps[p.doing]?.text ?? null) : null;
  const pct = (n: number) => (p.total === 0 ? 0 : (n / p.total) * 100);
  return {
    title: plan.title,
    done: p.done,
    total: p.total,
    now,
    donePct: pct(p.done),
    nowPct: now === null ? 0 : pct(1),
  };
}

/** The health line of a card. `tone` drives the colour; `null` text: nothing to say yet. */
export interface Health {
  tone: "ok" | "warn" | "none";
  text: string | null;
}

const plural = (n: number, word: string) => `${String(n)} ${word}${n === 1 ? "" : "s"}`;

/** What the card says about its handbook (the validator's findings, when the hub has arrived). */
export function healthOf(card: ProjectCard, hub: ProjectHub | null | undefined): Health {
  if (card.state === "none") return { tone: "none", text: null };
  if (card.state === "preFormat") return { tone: "warn", text: "Earlier handbook format: migrate" };
  if (card.state === "lite") return { tone: "ok", text: "Markdown pages: hints only" };
  if (hub === undefined || hub === null || hub.hub === null) return { tone: "none", text: null };
  const v = hub.hub.validation;
  const rules = (ids: string[]) => v.findings.filter((f) => ids.includes(f.rule)).length;
  if (v.errors > 0) return { tone: "warn", text: plural(v.errors, "error") };
  const missing = rules(["HB-W03"]);
  if (missing > 0) return { tone: "warn", text: `${plural(missing, "page")} not in the index` };
  if (rules(["HB-W04"]) > 0) return { tone: "warn", text: "Index older than the newest page" };
  if (v.warnings > 0) return { tone: "warn", text: plural(v.warnings, "warning") };
  return { tone: "ok", text: "Handbook up to date" };
}

/** "today 12:40", "yesterday", "3 days ago", or a date, for `ms` seen from `now`. */
export function ago(ms: number, now: number): string {
  if (ms <= 0) return "";
  const minutes = Math.floor((now - ms) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${String(minutes)} min ago`;
  const days = Math.floor((now - ms) / 86_400_000);
  if (days < 1) return `${String(Math.floor(minutes / 60))} h ago`;
  if (days === 1) return "yesterday";
  if (days < 14) return `${String(days)} days ago`;
  return new Date(ms).toISOString().slice(0, 10);
}
