/**
 * The account and usage panel's data. Account: `account/read` (plan title) and
 * `identity/read` (name, workspace), through a session. Usage: tokens and context fill from the
 * session's `session.tokenUsage`, `session.contextFill` (the stats event's `contextTokens` and
 * `contextWindow`, kept by the core) and Vibe's own `session.contextUsage` (token usage of the context).
 * Depends on: coreStore, protocol/core types. Called by: features/settings/AccountSettings,
 * settingsRoutes.
 * Invariants:
 * - Nothing is invented. Mistral does not share the plan quota left, so there is no such number.
 *   Only reported tokens and context fill are displayed.
 * - `contextWindow` in Vibe's stats is the point where it compacts (80% of the model's window), so the
 *   fill is "of N tokens until compaction", never "of the model's window".
 * - The email address is not read into the view (the screen has no use for it).
 */
import type { JsonValue, RateLimitsView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";

type Obj = { [key in string]?: JsonValue };

function asObject(value: JsonValue | undefined): Obj | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value : null;
}

function text(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

function count(value: JsonValue | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

/** What Vibe says about the account. */
export interface AccountView {
  /** `ready`, `missing_key`, `unauthorized` or `unavailable` (Vibe's words), or `unknown`. */
  status: string;
  /** The plan's title (for example "Pro"), when Vibe gives one. */
  plan: string | null;
  /** Vibe's `plan.kind`: `chat` (a plan key), `api` (a Studio key), `mistral_code`, or `null`. */
  planKind: string | null;
  name: string | null;
  workspace: string | null;
  organization: string | null;
  /** Why the account could not be read (plain words), or `null`. */
  problem: string | null;
}

/** Reads the account fields out of the two results. */
export function parseAccount(
  account: JsonValue | undefined,
  identity: JsonValue | undefined,
): AccountView {
  const view = asObject(asObject(account)?.["account"]);
  const plan = asObject(view?.["plan"]);
  const who = asObject(asObject(identity)?.["identity"]);
  const first = text(who?.["firstName"]);
  const last = text(who?.["lastName"]);
  return {
    status: text(view?.["status"]) ?? "unknown",
    plan: text(plan?.["title"]) ?? text(plan?.["name"]),
    planKind: text(plan?.["kind"]),
    name: [first, last].filter((x) => x !== null).join(" ") || null,
    workspace: text(asObject(who?.["workspace"])?.["name"]),
    organization: text(asObject(who?.["organization"])?.["name"]),
    problem: null,
  };
}

/**
 * Reads the account through `sessionId`; each half that fails leaves its fields empty, and a failed
 * `account/read` is reported with its reason (never swallowed).
 */
export async function readAccount(core: CoreController, sessionId: string): Promise<AccountView> {
  let problem: string | null = null;
  const call = async (method: string): Promise<JsonValue | undefined> => {
    try {
      const result = await core.run({ type: "call", method, params: { sessionId } });
      return result.type === "reply" ? result.result : undefined;
    } catch (thrown) {
      if (method === "account/read") problem = toCallError(thrown).error.message;
      return undefined;
    }
  };
  const [account, identity] = await Promise.all([call("account/read"), call("identity/read")]);
  return { ...parseAccount(account, identity), problem };
}

/**
 * The plan and key type in plain words, without making billing claims.
 * Known key kinds distinguish included plan usage from an API key; unknown kinds show the plan name alone.
 */
export function planInWords(view: Pick<AccountView, "plan" | "planKind">): string | null {
  if (view.plan === null) return null;
  // "[Subscription] Pro" -> "Pro"; "[API] Scale plan" -> "Scale plan".
  const name = view.plan.replace(/^\[[^\]]*\]\s*/, "").trim() || view.plan;
  const plan = /\bplan$/i.test(name) ? name : `${name} plan`;
  if (view.planKind === "chat") return `${plan} · this key uses your plan's included usage`;
  if (view.planKind === "api") return `${plan} · this is an API key`;
  return plan;
}

/** Tokens and context fill of one session. */
export interface UsageView {
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  /** Tokens in the context now, and the compaction threshold, when Vibe reported both. */
  contextTokens: number | null;
  untilCompaction: number | null;
}

/** Usage from a session object of the store (`session.tokenUsage`, `session.contextUsage`). */
export function usageOf(session: Record<string, JsonValue | undefined> | undefined): UsageView {
  const tokens = asObject(session?.["tokenUsage"]);
  const fill = asObject(session?.["contextFill"]);
  // The stats event's own reading is used when it is more than zero; Vibe's `session.contextUsage` (what the
  // current context weighs, kept fresh by session patches) fills in when the stats reading is zero or absent.
  const fromStats = count(fill?.["contextTokens"]);
  const fromSession = count(asObject(session?.["contextUsage"])?.["totalTokens"]);
  return {
    inputTokens: count(tokens?.["inputTokens"]),
    outputTokens: count(tokens?.["outputTokens"]),
    totalTokens: count(tokens?.["totalTokens"]),
    contextTokens: fromStats !== null && fromStats > 0 ? fromStats : (fromSession ?? fromStats),
    untilCompaction: count(fill?.["contextWindow"]),
  };
}

/** The share of the context used before compaction, 0 to 100, or `null` when not known. */
export function contextPercent(usage: UsageView): number | null {
  if (usage.contextTokens === null || usage.untilCompaction === null || usage.untilCompaction === 0)
    return null;
  return Math.min(100, Math.round((usage.contextTokens / usage.untilCompaction) * 100));
}

/**
 * The model's declared context window: `config.activeModel.maxContextLength` of the session's runtime snapshot, or
 * `null` when the model declares none (then the readout falls back to the compaction threshold).
 */
export function declaredWindow(runtime: JsonValue | undefined): number | null {
  const config = asObject(asObject(runtime)?.["config"]);
  const value = asObject(config?.["activeModel"])?.["maxContextLength"];
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

/** The context readout: the share of the declared window (else of the compaction threshold) and its explanation. */
export interface ContextReadout {
  percent: number;
  /** One sentence for the hover text and Settings. */
  explain: string;
}

/** The readout for `usage` and the session's runtime, or `null` while Vibe has not reported the numbers. */
export function contextReadout(
  usage: UsageView,
  runtime: JsonValue | undefined,
): ContextReadout | null {
  const window = declaredWindow(runtime);
  if (usage.contextTokens === null) return null;
  if (window !== null) {
    const compacts =
      usage.untilCompaction === null
        ? ""
        : ` Vibe compacts the conversation at ${formatTokens(usage.untilCompaction)}.`;
    return {
      percent: Math.min(100, Math.round((usage.contextTokens / window) * 100)),
      explain: `${formatTokens(usage.contextTokens)} of the model's ${formatTokens(window)}-token context window.${compacts}`,
    };
  }
  const percent = contextPercent(usage);
  if (percent === null || usage.untilCompaction === null) return null;
  return {
    percent,
    explain: `${formatTokens(usage.contextTokens)} of ${formatTokens(usage.untilCompaction)} tokens until Vibe compacts the conversation.`,
  };
}

/** A count with thousands separators ("12,345"); "not reported yet" for `null`. */
export function formatTokens(value: number | null): string {
  return value === null ? "not reported yet" : value.toLocaleString("en-US");
}

/** How long a rate-limit snapshot counts as current: the numbers are per minute. */
export const RATE_LIMITS_FRESH_MS = 60_000;

/** True while `limits` is recent enough to be shown as the current headroom (the status bar hides its warning otherwise). */
export function rateLimitsFresh(limits: RateLimitsView, nowMs: number): boolean {
  return nowMs - limits.observedAtMs <= RATE_LIMITS_FRESH_MS;
}

/** "hh:mm" in the user's locale clock (24 hour, no seconds). */
export function clockText(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}

/**
 * The Account row for the rate limit, or `null` when no answer has reported one (no placeholder text).
 * Fresh: "N of M tokens left, R of S requests left, seen hh:mm". Stale (older than a minute): the same
 * numbers marked "last seen hh:mm", never as current. A figure the answer did not carry is left out.
 */
export function rateLimitsText(limits: RateLimitsView | null, nowMs: number): string | null {
  if (limits === null) return null;
  const part = (left: number | null, of: number | null, what: string): string | null => {
    if (left === null) return null;
    return of === null
      ? `${formatTokens(left)} ${what} left`
      : `${formatTokens(left)} of ${formatTokens(of)} ${what} left`;
  };
  const parts = [
    part(limits.tokensRemaining, limits.tokensLimit, "tokens"),
    part(limits.requestsRemaining, limits.requestsLimit, "requests"),
  ].filter((x): x is string => x !== null);
  if (parts.length === 0) return null;
  const when = clockText(limits.observedAtMs);
  return `${parts.join(", ")} (${rateLimitsFresh(limits, nowMs) ? "seen" : "last seen"} ${when})`;
}

/** Below this share of the minute's tokens or requests left, the status bar warns. */
export const RATE_WARN_BELOW = 0.2;

/**
 * The status bar's rate-limit warning: the smaller of "tokens left" and "requests left" as a whole percent, when it is
 * under 20 % and the snapshot is fresh (at most a minute old); otherwise `null` (nothing is shown: no placeholder, and
 * old numbers are never presented as the current minute). A pair without its limit is left out.
 */
export function rateLimitWarning(limits: RateLimitsView | null, nowMs: number): number | null {
  if (limits === null || !rateLimitsFresh(limits, nowMs)) return null;
  const share = (left: number | null, of: number | null): number | null =>
    left === null || of === null || of <= 0 ? null : Math.max(0, left) / of;
  const shares = [
    share(limits.tokensRemaining, limits.tokensLimit),
    share(limits.requestsRemaining, limits.requestsLimit),
  ].filter((x): x is number => x !== null);
  if (shares.length === 0) return null;
  const least = Math.min(...shares);
  return least < RATE_WARN_BELOW ? Math.floor(least * 100) : null;
}
