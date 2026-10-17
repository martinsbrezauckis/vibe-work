/**
 * The Marketplace tab's pure model: groups, filters and the plain words for each entry's state, from the
 * core's `marketplace` view and (for MCP servers) the Connectors view's servers.
 * Depends on: the generated core types. Called by: the marketplace views, `marketRoutes`, tests.
 * Invariants: no side effects; every word shown about a vendor's rule is the catalog's (measured and quoted), never a
 * guess; an entry the core says is not installable on this computer is never offered as installable.
 */
import type { InstallProgress, MarketEntryView, MarketView, ServerView } from "../../protocol/core";

/** The filter chips. */
export type MarketFilter = "all" | "installed" | "updates";

export const MARKET_FILTERS: readonly { id: MarketFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "installed", label: "Installed" },
  { id: "updates", label: "Updates" },
];

/** Group ids in page order. */
export type GroupId = "plugin" | "pack" | "server";

const GROUPS: readonly { id: GroupId; label: string; line: string }[] = [
  {
    id: "plugin",
    label: "Plugins",
    line: "Tools that run on this computer, installed into Vibe Work's own folders.",
  },
  {
    id: "pack",
    label: "Skill packs",
    line: "Vendor guides that teach the agent a service, pinned to a reviewed version.",
  },
  {
    id: "server",
    label: "MCP servers",
    line: "Remote servers. Vibe signs in for you where the vendor allows it.",
  },
];

/** A status in a few words plus its tone. */
export interface Status {
  text: string;
  tone: "ok" | "warn" | "off" | "info";
}

/** The status of a pack or plugin. */
export function installStatus(e: MarketEntryView, installing: InstallProgress | null): Status {
  if (installing !== null && installing.id === e.entry.id)
    return { text: progressText(installing), tone: "info" };
  if (e.foreign) return { text: "A folder of this name exists (not ours)", tone: "warn" };
  if (e.off) return { text: "Turned off", tone: "off" };
  if (e.installed !== null) {
    if (e.installed.changed === true) return { text: "Changed on disk", tone: "warn" };
    if (e.installed.updateAvailable) return { text: "Update available", tone: "warn" };
    return { text: "Installed", tone: "ok" };
  }
  if (!e.installable) return { text: "Not available for this computer", tone: "off" };
  return { text: "Not installed", tone: "off" };
}

/** The server address in the form the Connectors view shows it (scheme, host, path; no trailing slash). */
function sameAddress(a: string, b: string | null): boolean {
  if (b === null) return false;
  const norm = (u: string) => u.replace(/\/+$/, "").toLowerCase();
  return norm(a) === norm(b);
}

/** The server Vibe has for a catalog server entry, when it was added. */
export function addedServer(e: MarketEntryView, servers: readonly ServerView[]): ServerView | null {
  const url = e.entry.server?.url;
  if (url === undefined) return null;
  return servers.find((s) => sameAddress(url, s.address)) ?? null;
}

/** The status of a remote server: the measured access class, or Vibe's live state once added. */
export function serverStatus(e: MarketEntryView, servers: readonly ServerView[]): Status {
  const access = e.entry.server?.access;
  const added = addedServer(e, servers);
  if (added !== null) {
    if (added.off) return { text: "Turned off", tone: "off" };
    if (added.key?.grant === "none") return { text: "Key not allowed", tone: "warn" };
    if (added.status === "connected" || (added.status === "enabled" && added.tools.length > 0))
      return { text: "Connected", tone: "ok" };
    if (added.status === "needs_auth") return { text: "Needs sign-in", tone: "warn" };
    return { text: "Added", tone: "info" };
  }
  switch (access) {
    case "signIn":
      return { text: "Vibe signs in for you", tone: "info" };
    case "approvedOnly":
      return { text: "Vendor allows approved apps only", tone: "warn" };
    case "ownApp":
      return { text: "Needs your own app registration", tone: "warn" };
    case "key":
      return { text: "Uses your API key", tone: "info" };
    default:
      return { text: "Unknown", tone: "off" };
  }
}

/** The status of any entry. */
export function statusOf(
  e: MarketEntryView,
  installing: InstallProgress | null,
  servers: readonly ServerView[],
): Status {
  return e.entry.kind === "server" ? serverStatus(e, servers) : installStatus(e, installing);
}

/** Plain words for an install in progress. */
export function progressText(p: InstallProgress): string {
  const mb = (n: number | null) => (n === null ? null : `${String(Math.round(n / 1048576))} MB`);
  switch (p.phase) {
    case "download": {
      const done = mb(p.done);
      return p.what === ""
        ? "Starting…"
        : `Downloading ${p.what}${done === null ? "" : ` (${done})`}`;
    }
    case "unpack":
      return `Unpacking ${p.what}`;
    case "setup":
      return p.what;
    case "finish":
      return "Finishing…";
    case "failed":
      return "Install failed";
    case "cancelled":
      return "Stopped";
    default:
      return "Installed";
  }
}

/** True when the entry passes the filter and the search text. */
export function matches(
  e: MarketEntryView,
  filter: MarketFilter,
  search: string,
  servers: readonly ServerView[],
): boolean {
  if (filter === "installed") {
    const installed =
      e.entry.kind === "server" ? addedServer(e, servers) !== null : e.installed !== null;
    if (!installed) return false;
  }
  if (filter === "updates" && e.installed?.updateAvailable !== true) return false;
  const q = search.trim().toLowerCase();
  if (q === "") return true;
  return [e.entry.name, e.entry.publisher, e.entry.summary, e.entry.category]
    .join(" ")
    .toLowerCase()
    .includes(q);
}

/** One group of cards. */
export interface Group {
  id: GroupId;
  label: string;
  line: string;
  entries: MarketEntryView[];
}

/** The groups shown for the view, filter and search (empty groups left out). */
export function marketGroups(
  view: MarketView | null,
  filter: MarketFilter,
  search: string,
  servers: readonly ServerView[],
): Group[] {
  if (view === null) return [];
  return GROUPS.map((g) => ({
    ...g,
    entries: view.entries.filter(
      (e) => e.entry.kind === g.id && matches(e, filter, search, servers),
    ),
  })).filter((g) => g.entries.length > 0);
}

/** Counts per filter (for the chips). */
export function marketCounts(
  view: MarketView | null,
  servers: readonly ServerView[],
): Record<MarketFilter, number> {
  const all = view?.entries ?? [];
  return {
    all: all.length + (view?.discovered.length ?? 0),
    installed:
      all.filter((e) => matches(e, "installed", "", servers)).length +
      (view?.discovered.length ?? 0),
    updates: all.filter((e) => matches(e, "updates", "", servers)).length,
  };
}

/** The category heading for an id (the core sends the list). */
export function categoryLabel(view: MarketView | null, id: string): string {
  return view?.categories.find((c) => c.id === id)?.label ?? id;
}

/** What a pack or plugin contains, in a line ("6 skills, 1 server"). */
export function contents(e: MarketEntryView): string {
  const parts: string[] = [];
  const pack = e.entry.pack;
  const plugin = e.entry.plugin;
  const skills = pack
    ? pack.files.filter((f) => f.to.endsWith("/SKILL.md")).length
    : plugin
      ? Object.keys(plugin.bundled).filter((k) => k.endsWith("/SKILL.md")).length
      : 0;
  const servers = Object.keys(pack?.mcpServers ?? plugin?.mcpServers ?? {}).length;
  if (skills > 0) parts.push(`${String(skills)} skill${skills === 1 ? "" : "s"}`);
  if (servers > 0) parts.push(`${String(servers)} server${servers === 1 ? "" : "s"}`);
  return parts.join(", ");
}

/** The pinned version line ("version 0.0.83", "upstash/context7 at feef1b2"). */
export function pinLine(e: MarketEntryView): string | null {
  const pack = e.entry.pack;
  if (pack?.source) return `${pack.source.repo} at ${pack.source.commit.slice(0, 12)}`;
  if (pack) return null;
  const plugin = e.entry.plugin;
  if (plugin) return `version ${plugin.version}, ${plugin.runtime.name} ${plugin.runtime.version}`;
  return null;
}
