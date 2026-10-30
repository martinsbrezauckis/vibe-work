/**
 * Which core events deserve a visible notification, and what they say. Pure: the controller feeds
 * the result to the toast store. Session badges and the unread dots show every change; toasts are
 * for things the user would otherwise miss.
 * Asks follow the bell rule by who decided.
 * Depends on: protocol/core types, askBell. Called by: coreStore.
 */
import { sessionTitle } from "./sessionSummary";
import type { CoreEvent } from "../protocol/core";
import { askWords, securityGuard, securityLine } from "./askBell";
import type { CoreState } from "./coreModel";

/** The start of the tool result the core returns for a call the user said No to. */
const USER_DECLINED = /^Not run: the user declined this\./;

/** Severity of a toast (drives its marker colour and whether it stays until dismissed). */
type ToastKind = "info" | "warn" | "error";

/** Reasons an entry may be retained in the bell; ordinary feedback has none. */
export const BELL_KINDS = ["approval", "finished", "failure", "security", "worktrees"] as const;
export type BellKind = (typeof BELL_KINDS)[number];

/** Fail closed for unclassified feedback and unknown kinds, including untyped callers. */
export function isBellKind(kind: unknown): kind is BellKind {
  return BELL_KINDS.some((allowed) => allowed === kind);
}

/** What to show. */
export interface ToastSpec {
  kind: ToastKind;
  /** A redacted guard decision, grouped by the toast store. */
  guardRefusal?: GuardRefusal;
  /** The mapped decision line is more specific than a grouped count. */
  preserveSecurityText?: boolean;
  /** Retained security group, bounded to the latest decisions. */
  security?: SecurityGroup;
  /** Explicit attention reason, required for bell history; severity alone never admits it. */
  bellKind?: BellKind;
  text: string;
  /** Session to open from the retained notification, when applicable. */
  sessionId?: string;
  /** Opens the existing sessions list and orphan copies from a worktree count notice. */
  action?: "sessions";
  /** Discovered outside the catalog; the bell offers the same owned-folder controls as Tools. */
  plugin?: { name: string; path: string; managed: boolean };
  /** Rust-owned session-log finding; the bell action uses its opaque handle. */
  secretLeak?: { findingId: string };
}

/** Retained counts include decisions beyond the disclosure's visible lines. */
interface SecurityGroup {
  total: number;
  counts: Partial<Record<GuardKind, number>>;
  refusals: readonly GuardRefusal[];
}
type CoreRefusal = Extract<CoreEvent, { type: "guardRefused" }>;
/** The core's guards plus the window's own "Never setting" line for an ask a Never refused. */
export type GuardKind = CoreRefusal["guard"] | "never";
/** One Security group line: the core's guard refusal, or an ask outcome worded the same way. */
type GuardRefusal = Omit<CoreRefusal, "guard"> & { guard: GuardKind };
export const GUARD_NAMES: Record<GuardKind, string> = {
  ask: "Permission prompts",
  // Not "Ask": that is the canonical name of an access mode (vocabulary check).
  never: "Never setting",
  git: "Git guard",
  folder: "Folder guard",
  network: "Network guard",
  process: "Process guard",
  safetyRule: "Safety rule",
  protectedFolderDeclined: "Protected-folder change declined",
};

/** The toast for `event`, or `null` when it needs none. `core` (the model after the event) words asks. */
export function notificationFor(event: CoreEvent, core?: CoreState): ToastSpec | null {
  switch (event.type) {
    case "askClosed": {
      // Who decided sets where the outcome shows: your own click and expiry add nothing here (the
      // audit row shows the click; the waiting entry turns into the timeout text, notifyStore.ts).
      const record = event.record;
      const words = askWords(core, record);
      const what = securityLine(record, words);
      if (what === null) return null;
      const { ask } = record;
      const session = ask.session === null ? {} : { sessionId: ask.session };
      // An allow is not a stopped action: it is its own Security entry, outside the "stopped N actions" group.
      if (record.outcome.type === "allowed")
        return {
          kind: "info",
          bellKind: "security",
          text: `“${words.session}”: ${what}.`,
          ...session,
        };
      return {
        kind: "warn",
        bellKind: "security",
        text: `“${words.session}”: ${what}.`,
        ...session,
        preserveSecurityText: record.outcome.type === "refused" && record.outcome.by === "debugApi",
        guardRefusal: {
          type: "guardRefused",
          sessionId: ask.session ?? "",
          title: words.session,
          guard: securityGuard(record),
          what,
          stepId: ask.id,
          at: record.closedAt,
          refusalId: `ask:${ask.id}`,
        },
      };
    }

    case "guardRefused": {
      const title =
        event.title.trim() === "" || event.title === event.sessionId
          ? sessionTitle(core?.sessions[event.sessionId]?.session)
          : event.title;
      return {
        kind: "warn",
        bellKind: "security",
        sessionId: event.sessionId,
        guardRefusal: { ...event, title },
        preserveSecurityText: USER_DECLINED.test(event.what),
        // The declined result is written for the agent; the person only needs to know their No was kept.
        text: USER_DECLINED.test(event.what)
          ? `No was recorded in “${title}”.`
          : `${event.what.replace(/[.]+$/, "")}${event.what.endsWith("…") ? "" : "."} In “${title}”.`,
      };
    }

    // A protected change's outcome follows the ask rule above:
    // your own No leaves only the audit row.
    case "alert":
      return {
        kind: event.kind === "temporaryCleanupRecovered" ? "info" : "warn",
        bellKind:
          event.kind === "bypassToolPermissions" ||
          event.kind === "accessModeLowered" ||
          event.kind === "accessModeUnverified" ||
          event.kind === "networkSettingsRejected"
            ? "security"
            : "failure",
        text: event.message,
        ...(event.sessionId === null ? {} : { sessionId: event.sessionId }),
      };
    case "refused":
      // Declining a native dialog is the user's own answer, not a refusal worth a toast.
      if (event.code === "confirmationDeclined" || event.code === "accessModeUnverified")
        return null;
      // A failed key or vault operation (wrong password, a key Mistral refuses, "no key service in this
      // run") is shown inline by the key screens or the affected feature, and an external client gets it in its answer: a second,
      // raw toast would only repeat it.
      if (event.code === "keyService") return null;
      // The originating control owns UI errors. Retain an external agent's refusal as a security event.
      if (event.origin !== "debug-api") return null;
      return {
        kind: "warn",
        bellKind: "security",
        text: `Debug API request refused: ${event.reason}`,
      };
    case "server":
      if (event.status.state === "restarting") {
        return { kind: "warn", bellKind: "failure", text: "Vibe stopped and is restarting" };
      }
      if (event.status.state === "dead") {
        return {
          kind: "error",
          bellKind: "failure",
          text: `Vibe could not be restarted: ${event.status.reason}`,
        };
      }
      return null;
    case "debugApi":
      // The enabled flag is already visible in the footer and Settings next to its control.
      return null;
    default:
      return null;
  }
}
