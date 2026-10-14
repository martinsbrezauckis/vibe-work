/**
 * Which core events deserve a visible notification, and what they say. Pure: the controller feeds
 * the result to the toast store. Session badges and the unread dots show every change; toasts are
 * for things the user would otherwise miss.
 * Depends on: protocol/core types. Called by: coreStore.
 */
import type { CoreEvent } from "../protocol/core";

/** Severity of a toast (drives its marker colour and whether it stays until dismissed). */
export type ToastKind = "info" | "warn" | "error";

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
  /** Explicit attention reason, required for bell history; severity alone never admits it. */
  bellKind?: BellKind;
  text: string;
  /** Session to open from the retained notification, when applicable. */
  sessionId?: string;
  /** Opens the existing sessions list and orphan copies from a worktree count notice. */
  action?: "sessions";
}

/** The toast for `event`, or `null` when it needs none. */
export function notificationFor(event: CoreEvent): ToastSpec | null {
  switch (event.type) {
    case "alert":
      return {
        kind: "warn",
        bellKind:
          event.kind === "bypassToolPermissions" ||
          event.kind === "accessModeLowered" ||
          event.kind === "accessModeUnverified"
            ? "security"
            : "failure",
        text: event.message,
        ...(event.sessionId === null ? {} : { sessionId: event.sessionId }),
      };
    case "refused":
      // Declining a native dialog is the user's own answer, not a refusal worth a toast.
      if (event.code === "confirmationDeclined") return null;
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
