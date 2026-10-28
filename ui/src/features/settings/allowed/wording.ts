/**
 * The words of the Allowed without asking page and the Safety summary line: times
 * ("added 14:02", "last 2 hours ago"), counts, the kind of a session answer, and the per-kind status and
 * confirmation texts. Pure functions of their inputs (`now` is passed in, so tests are exact).
 * Depends on: nothing. Called by: allowed/entries, allowed/view, allowed/store.
 */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

/** "1 time" / "6 times" (any noun: `plural(3, "rule")` = "3 rules"). */
export function plural(n: number, noun: string, many = `${noun}s`): string {
  return `${String(n)} ${n === 1 ? noun : many}`;
}

/** The local clock time, as the cards show it ("14:02"). */
function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function sameDay(a: number, b: number): boolean {
  const x = new Date(a);
  const y = new Date(b);
  return (
    x.getFullYear() === y.getFullYear() &&
    x.getMonth() === y.getMonth() &&
    x.getDate() === y.getDate()
  );
}

/** Whole calendar days between two instants (local time). */
function daysBetween(earlier: number, later: number): number {
  const a = new Date(earlier);
  const b = new Date(later);
  const start = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  return Math.round((start(b) - start(a)) / (24 * HOUR));
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/**
 * A day as "1 Aug" (the year only when it is not this year: "1 Aug 2025"). Spelled here, not by Intl: the UI is
 * English and Intl's short month differs between runtimes ("Sep" / "Sept").
 */
export function shortDate(ms: number, now: number): string {
  const d = new Date(ms);
  const day = `${String(d.getDate())} ${MONTHS[d.getMonth()] ?? ""}`;
  return d.getFullYear() === new Date(now).getFullYear()
    ? day
    : `${day} ${String(d.getFullYear())}`;
}

/** When a session answer was added: "14:02" today, else "17 Oct 14:02". */
export function sessionAdded(ms: number, now: number): string {
  return sameDay(ms, now) ? clock(ms) : `${shortDate(ms, now)} ${clock(ms)}`;
}

/** A last use inside a session: "at 14:31" today, else "on 17 Oct at 14:31". */
export function sessionLast(ms: number, now: number): string {
  return sameDay(ms, now) ? `at ${clock(ms)}` : `on ${shortDate(ms, now)} at ${clock(ms)}`;
}

/** A last use of a lasting grant: "just now", "5 minutes ago", "2 hours ago", "yesterday", "3 days ago", "on 1 Aug". */
export function relative(ms: number, now: number): string {
  const ago = now - ms;
  if (ago < MINUTE) return "just now";
  if (ago < HOUR) return `${plural(Math.floor(ago / MINUTE), "minute")} ago`;
  const days = daysBetween(ms, now);
  if (days <= 0) return `${plural(Math.floor(ago / HOUR), "hour")} ago`;
  if (days === 1) return "yesterday";
  if (days < 30) return `${String(days)} days ago`;
  return `on ${shortDate(ms, now)}`;
}

/**
 * How often an entry was used: "not used yet"; "used 6 times, last at 14:31" (`last` words the time); a use count
 * without a recorded time says so ("used 3 times, last use not recorded yet": grants from before Vibe Work kept it).
 */
export function usage(uses: number, lastMs: number | null, last: (ms: number) => string): string {
  if (uses === 0 && lastMs === null) return "not used yet";
  const count = `used ${plural(uses, "time")}`;
  return lastMs === null ? `${count}, last use not recorded yet` : `${count}, last ${last(lastMs)}`;
}

/** The shape of what a session grant answers, for its detail line. */
export interface GrantShape {
  tool: string;
  everyCall: boolean;
  permissions: readonly { scope: string }[];
}

/** The kind of a session answer ("Shell command", "Background command", "MCP tool", "Outside the folder"…). */
export function sessionKind(grant: GrantShape): string {
  if (grant.tool === "mcp_vw.process_start") return "Background command";
  if (grant.tool === "mcp_vw.secret_run") return "Command with secrets";
  const scopes = grant.permissions.map((p) => p.scope);
  if (scopes.includes("outside_directory")) return "Outside the folder";
  if (scopes.includes("command_pattern")) return "Shell command";
  if (scopes.includes("file_pattern")) return "File edit";
  if (scopes.includes("url_pattern")) return "Web address";
  return grant.tool.includes(".") ? "MCP tool" : "Tool";
}

/** The last segment of a folder path ("city-app" for "/home/anna/proj/city-app"). */
export function folderName(folder: string): string {
  const parts = folder.split(/[\\/]+/).filter((p) => p !== "");
  return parts[parts.length - 1] ?? folder;
}

/** The status line after one entry of a kind is revoked. */
export const REVOKED_ONE = {
  session: "Revoked. Vibe asks again next time.",
  rule: "Revoked. New sessions ask again. A session that is open now may keep using it until it ends.",
  vault: "Revoked.",
} as const;

/** The status line after a group's bulk revoke. */
export function revokedMany(kind: "session" | "rule" | "vault", n: number): string {
  if (kind === "rule") return `Revoked ${String(n)}. Vibe asks again before each.`;
  if (kind === "session") return `Revoked ${String(n)}. Vibe asks again next time.`;
  return `Revoked ${String(n)}.`;
}
