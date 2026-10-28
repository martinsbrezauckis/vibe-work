/**
 * The Connectors page's derived data: one tile per Mistral-hosted connector with a status in
 * plain words, grouped by category, filtered and searched; and the servers' status words. Pure.
 * Depends on: model/meta, the generated connector types. Called by: the connector views, the
 * `ui/connectors` route, tests.
 * Status words follow Vibe's own states: `readiness` (`ready`, `needs_auth`, `needs_setup`, `unavailable`)
 * plus the off flag; when a session's state was read, its status (`connected`, `disabled`,...) wins.
 */
import type { ConnectorView, ConnectorsView, ServerView } from "../../../protocol/core";
import { CATEGORIES, type CategoryId, type ConnectorMeta, metaOf } from "./meta";

/** A tile's state. */
export type TileStatus = "connected" | "signin" | "setup" | "unavailable" | "off";

/** What the page shows for one connector. */
export interface Tile {
  alias: string;
  meta: ConnectorMeta;
  status: TileStatus;
  /** The status in words. */
  statusText: string;
  /** Vibe's reason (unavailable), else null. */
  reason: string | null;
  connector: ConnectorView;
}

/** The page filter. */
export type Filter = "all" | "connected" | "signin" | "off";

/** Filter chips in order. */
export const FILTERS: readonly { id: Filter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "connected", label: "Connected" },
  { id: "signin", label: "Needs sign-in" },
  { id: "off", label: "Off" },
];

const STATUS_TEXT: Readonly<Record<TileStatus, string>> = {
  connected: "Connected",
  signin: "Needs sign-in",
  setup: "Needs setup at Mistral",
  unavailable: "Unavailable",
  off: "Off",
};

/** The tile status of `c`. */
export function tileStatus(c: ConnectorView): TileStatus {
  if (c.off || c.sessionStatus === "disabled") return "off";
  const live = c.sessionStatus;
  if (live === "connected" || (live === null && c.readiness === "ready")) return "connected";
  const state = live ?? c.readiness;
  if (state === "needs_auth") return "signin";
  if (state === "needs_setup") return "setup";
  return "unavailable";
}

/**
 * Vibe 3's `needs_connect`: a connector that needs no sign-in, connected by one call (`connector_catalog/connect`)
 * that the gate does not offer yet (roadmap: connectors-that-need-no-sign-in). Said plainly instead of "Unavailable".
 */
function noSignIn(c: ConnectorView): boolean {
  return (c.sessionStatus ?? c.readiness) === "needs_connect";
}

const NO_SIGN_IN_REASON = "This connector needs no sign-in, but Vibe Work cannot connect it yet.";

/** The tile of one connector. */
export function tileOf(c: ConnectorView): Tile {
  const status = tileStatus(c);
  return {
    alias: c.alias,
    meta: metaOf(c.alias, c.displayName),
    status,
    statusText: noSignIn(c) ? "Not connected" : STATUS_TEXT[status],
    reason: noSignIn(c)
      ? NO_SIGN_IN_REASON
      : status === "unavailable"
        ? (c.diagnostic ?? "Vibe gives no reason.")
        : null,
    connector: c,
  };
}

/** True when `tile` passes `filter` and `search` (name, line, category, alias; case-insensitive). */
function matches(tile: Tile, filter: Filter, search: string): boolean {
  if (filter === "connected" && tile.status !== "connected") return false;
  if (filter === "signin" && tile.status !== "signin") return false;
  if (filter === "off" && tile.status !== "off") return false;
  const q = search.trim().toLowerCase();
  if (q === "") return true;
  const label = CATEGORIES.find((c) => c.id === tile.meta.category)?.label ?? "";
  return [tile.meta.name, tile.meta.line, label, tile.alias].some((t) =>
    t.toLowerCase().includes(q),
  );
}

/** One category group. */
export interface Group {
  id: CategoryId;
  label: string;
  tiles: Tile[];
}

/** The tiles of `view` grouped in category order, empty groups left out, tiles by name. */
export function groups(view: ConnectorsView | null, filter: Filter, search: string): Group[] {
  const tiles = (view?.connectors ?? []).map(tileOf).filter((t) => matches(t, filter, search));
  return CATEGORIES.map((c) => ({
    id: c.id,
    label: c.label,
    tiles: tiles
      .filter((t) => t.meta.category === c.id)
      .sort((a, b) => a.meta.name.localeCompare(b.meta.name)),
  })).filter((g) => g.tiles.length > 0);
}

/** How many connectors each filter would show. */
export function counts(view: ConnectorsView | null): Record<Filter, number> {
  const tiles = (view?.connectors ?? []).map(tileOf);
  return {
    all: tiles.length,
    connected: tiles.filter((t) => t.status === "connected").length,
    signin: tiles.filter((t) => t.status === "signin").length,
    off: tiles.filter((t) => t.status === "off").length,
  };
}

/** A server's state in words. */
export function serverStatusText(s: ServerView): string {
  if (s.off) return "Off";
  // a key server whose key is not allowed now (or the vault is locked) cannot connect.
  if (s.key?.grant === "none") return "Key not allowed";
  if (s.key?.grant === "locked") return "Vault locked";
  switch (s.status) {
    case null:
      return s.signIn === "oauth" ? "Added (status shows once a session is open)" : "Added";
    case "connected":
      return "Connected";
    // Vibe 2.26.1 says `enabled` for a working server without OAuth (`app_server/_projection.py`): connected once
    // its tools are known.
    case "enabled":
      return s.tools.length > 0 ? "Connected" : "Added";
    case "needs_auth":
      return "Needs sign-in";
    case "disabled":
      return "Off";
    case "needs_setup":
      return "Needs setup";
    case "unavailable":
      return "Unavailable";
    case "needs_connect":
      return "Not connected";
    default:
      return s.status;
  }
}

/** The status class of a server (same palette as tiles). */
export function serverTone(s: ServerView): TileStatus {
  if (s.off || s.status === "disabled") return "off";
  if (s.key?.grant === "none" || s.key?.grant === "locked") return "signin";
  if (s.status === "connected" || (s.status === "enabled" && s.tools.length > 0))
    return "connected";
  if (s.status === "needs_auth" || s.status === null) return "signin";
  return "unavailable";
}
