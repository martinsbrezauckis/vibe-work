/**
 * Query modes, fixed grouping and row reconciliation for the global palette.
 * Depends on: paletteTypes. Called by: usePaletteResults, CommandPalette.
 * Invariants: only an explicit slash searches actions; surviving rows keep their order and receive fresh content.
 */
import type { Kind, Result } from "./paletteTypes";

/** A slash asks for actions; ordinary words ask for content. An empty field keeps the command list. */
export function paletteQuery(query: string): { mode: "empty" | "text" | "action"; text: string } {
  const text = query.trim();
  return text.startsWith("/")
    ? { mode: "action", text: text.slice(1).trim() }
    : { mode: text === "" ? "empty" : "text", text };
}

/** Results and filters retain the same type order, regardless of match scores or arrival time. */
export const TEXT_GROUPS: readonly Kind[] = ["session", "message", "file", "setting"];
/** Every result, before the per-group display cap is applied. */
export type ResultGroups = Record<Kind, readonly Result[]>;
/** The selected type, or all types. */
export type Filter = "all" | Kind;
/** A visible result, plus its group and the count hidden after its last row. */
export interface PaletteRow {
  result: Result;
  group: Kind;
  more: number;
}

/** Keeps surviving identities in place, replaces their content, and appends new identities in answer order. */
export function reconcileRows<T>(
  previous: readonly T[],
  incoming: readonly T[],
  id: (row: T) => string,
): T[] {
  const fresh = new Map(incoming.map((row) => [id(row), row]));
  const rows: T[] = [];
  for (const old of previous) {
    const key = id(old);
    const row = fresh.get(key);
    if (row !== undefined) {
      rows.push(row);
      fresh.delete(key);
    }
  }
  return [...rows, ...fresh.values()];
}

/** Flattens fixed groups; text searches show five per type until that type's filter is chosen. */
export function paletteRows(
  groups: ResultGroups,
  mode: ReturnType<typeof paletteQuery>["mode"],
  filter: Filter,
): PaletteRow[] {
  const order: readonly Kind[] =
    mode === "empty" ? ["action", "session"] : mode === "action" ? ["action"] : TEXT_GROUPS;
  const rows: PaletteRow[] = [];
  for (const group of order) {
    if (mode === "text" && filter !== "all" && filter !== group) continue;
    const all = groups[group];
    const shown = mode === "text" && filter === "all" ? all.slice(0, 5) : all;
    shown.forEach((result, at) =>
      rows.push({ result, group, more: at === shown.length - 1 ? all.length - shown.length : 0 }),
    );
  }
  return rows;
}
