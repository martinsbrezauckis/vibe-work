/**
 * The bulk confirmation of the Allowed without asking page: title, what happens, up to five
 * entries and "and {n} more.", and the confirming button, per leaf list (a session's answers, the earlier rules, one
 * vault part). While Find or Project filters, it says "these {n}" and holds only the matches.
 * Depends on: entries, view (LeafView), wording. Called by: allowed/store. Pure.
 */
import type { AllowedEntry } from "./entries";
import type { LeafView } from "./view";
import { plural } from "./wording";

/** At most this many entries are listed in the dialog. */
const DIALOG_LIST = 5;

/** The open confirmation. */
export interface RevokeAllDialog {
  /** The leaf it revokes from. */
  leaf: string;
  /** The entry keys it revokes (exactly these, also when the list changes meanwhile). */
  keys: readonly string[];
  title: string;
  body: string;
  list: readonly string[];
  /** "and 41 more." or null. */
  more: string | null;
  confirm: string;
}

function lead(filtering: boolean, n: number): string {
  return filtering ? `these ${String(n)}` : `all ${String(n)}`;
}

/** The words of one leaf's confirmation. */
function wordsFor(
  leaf: LeafView,
  n: number,
  filtering: boolean,
): { title: string; body: string; confirm: string } {
  const all = lead(filtering, n);
  if (leaf.key === "earlier")
    return {
      title: `Revoke ${all} earlier “Always allow” ${n === 1 ? "rule" : "rules"}?`,
      body: "Vibe asks you again before each of these, in every project. They cannot be added back. Sessions that are open now keep them until they end.",
      confirm: `Revoke ${plural(n, "rule")}`,
    };
  if (leaf.key.startsWith("sessions/"))
    return {
      title: `Revoke ${all} ${n === 1 ? "answer" : "answers"} in ${leaf.title}?`,
      body: "Vibe asks you again before each of these in this session.",
      confirm: `Revoke ${plural(n, "answer")}`,
    };
  if (leaf.key === "vault/fills")
    return {
      title: `Revoke ${all} ${n === 1 ? "site" : "sites"} where a login may be filled?`,
      body: "The vault asks you again before it fills a login on each of these sites.",
      confirm: `Revoke ${plural(n, "site")}`,
    };
  return {
    title: `Revoke ${all} MCP ${n === 1 ? "server" : "servers"} that get a key?`,
    body: "The vault asks you again before it gives each of these servers its key.",
    confirm: `Revoke ${plural(n, "server")}`,
  };
}

/** The confirmation for revoking every matching entry of `leaf`. */
export function revokeAllDialog(leaf: LeafView, filtering: boolean): RevokeAllDialog {
  const entries: readonly AllowedEntry[] = leaf.entries;
  const n = entries.length;
  const rest = n - DIALOG_LIST;
  return {
    leaf: leaf.key,
    keys: entries.map((e) => e.key),
    ...wordsFor(leaf, n, filtering),
    list: entries.slice(0, DIALOG_LIST).map((e) => e.line),
    more: rest > 0 ? `and ${String(rest)} more.` : null,
  };
}
