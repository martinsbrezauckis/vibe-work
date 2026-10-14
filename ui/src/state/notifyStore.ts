/**
 * The notification controller: keeps the level setting, watches the core model for changes that
 * call for an OS notification (`osNotify.ts`), asks the host to show them, and records what it
 * asked for (the `notifications` UI-control route reads that record in tests).
 * Depends on: createStore, coreModel, osNotify, protocol/client types, persist. Called by: main.tsx
 * (creates and starts it), the Settings page (level, status), shellUiControl (route).
 * Invariants:
 * - Only the host can show an OS notification; this module never touches a browser Notification.
 * - A host failure (no notification service, rate cap) is kept as `error` for the settings row:
 *   "Off in system settings" when the host reports the OS refused; it never throws to the caller.
 * - `focusOverride` (a test hook behind the Debug API route) replaces the real focus signal; `null`
 *   means real. It only changes whether a notification is shown, never what it says.
 * - The log is bounded (`LOG_MAX`).
 */
import type { CoreClient, OsNotification } from "../protocol/client/types";
import { toCallError } from "../protocol/client/types";
import type { CoreState } from "./coreModel";
import { type Store, createStore } from "./createStore";
import type { ToastSpec } from "./notifications";
import {
  DEFAULT_NOTIFY_LEVEL,
  fullTextFor,
  type NotifyLevel,
  NOTIFY_LEVELS,
  shouldNotify,
  textFor,
  transitions,
} from "./osNotify";

const LEVEL_KEY = "vw.notifyLevel";
const LOG_MAX = 50;

/** Notification settings and what happened. */
export interface NotifyState {
  level: NotifyLevel;
  /** Test override for window focus; `null` = the real signal. */
  focusOverride: boolean | null;
  /** The last host failure, or `null`. */
  error: string | null;
  /** Notifications requested from the host, newest last. */
  log: readonly OsNotification[];
}

function loadLevel(): NotifyLevel {
  try {
    const stored = localStorage.getItem(LEVEL_KEY);
    return NOTIFY_LEVELS.find((l) => l === stored) ?? DEFAULT_NOTIFY_LEVEL;
  } catch {
    return DEFAULT_NOTIFY_LEVEL;
  }
}

/** The real focus signal: the document is visible and has focus. */
export function documentFocused(): boolean {
  return typeof document !== "undefined" && document.hasFocus() && !document.hidden;
}

/** The controller. */
export function createNotifier(options: { persist?: boolean } = {}) {
  const store: Store<NotifyState> = createStore<NotifyState>({
    level: options.persist === false ? DEFAULT_NOTIFY_LEVEL : loadLevel(),
    focusOverride: null,
    error: null,
    log: [],
  });
  return {
    store,
    /** Sets the level (and remembers it). */
    setLevel(level: NotifyLevel): void {
      store.set((s) => ({ ...s, level }));
      if (options.persist === false) return;
      try {
        localStorage.setItem(LEVEL_KEY, level);
      } catch {
        // Not remembered; the setting still applies for this run.
      }
    },
    /** Overrides the focus signal (`null` = real). */
    setFocusOverride(focused: boolean | null): void {
      store.set((s) => ({ ...s, focusOverride: focused }));
    },
    /**
     * Watches `core` and notifies through its client. `openSession` returns the session open in the
     * window. `record` saves full text for the in-app notification panel, when available.
     * Returns the function that stops watching.
     */
    watch(
      core: { store: Store<CoreState>; client: CoreClient },
      openSession: () => string | null,
      record?: (spec: ToastSpec) => number,
      forget?: (id: number) => void,
    ): () => void {
      let last = core.store.get();
      const seenCallbacks = new Set<string>();
      const retainedCallbacks = new Map<
        string,
        { id: number; sessionId: string; callbackId: string }
      >();
      return core.store.subscribe(() => {
        const next = core.store.get();
        const found = transitions(last, next);
        for (const [key, retained] of retainedCallbacks) {
          if (
            next.answeredBy[retained.callbackId] !== undefined ||
            !next.sessions[retained.sessionId]?.openCallbackIds.includes(retained.callbackId)
          ) {
            forget?.(retained.id);
            retainedCallbacks.delete(key);
          }
        }
        last = next;
        const state = store.get();
        const focused = state.focusOverride ?? documentFocused();
        for (const t of found) {
          const callbackKey =
            t.callbackId === undefined ? null : `${t.sessionId}\u0000${t.callbackId}`;
          if (callbackKey !== null) {
            if (seenCallbacks.has(callbackKey)) continue;
            seenCallbacks.add(callbackKey);
          }
          const announce = shouldNotify(state.level, t, { focused, openSessionId: openSession() });
          // Waiting cards and stopped work belong in the bell even while the user is looking.
          // Finished turns are retained only when the user's level and attention warrant it.
          if (t.kind === "finished" && !announce) continue;
          const notificationId = record?.({
            kind: t.kind === "error" ? "error" : "info",
            bellKind:
              t.kind === "permission" ? "approval" : t.kind === "error" ? "failure" : "finished",
            text: fullTextFor(t),
            sessionId: t.sessionId,
          });
          const notification: OsNotification = {
            sessionId: t.sessionId,
            ...(notificationId !== undefined &&
            Number.isInteger(notificationId) &&
            notificationId > 0 &&
            notificationId <= 0xffff_ffff
              ? { notificationId }
              : {}),
            ...textFor(t),
          };
          if (callbackKey !== null && notificationId !== undefined && t.callbackId !== undefined)
            retainedCallbacks.set(callbackKey, {
              id: notificationId,
              sessionId: t.sessionId,
              callbackId: t.callbackId,
            });
          if (!announce) continue;
          store.set((s) => ({ ...s, log: [...s.log, notification].slice(-LOG_MAX) }));
          core.client.notifyOs(notification).then(
            () => {
              store.set((s) => (s.error === null ? s : { ...s, error: null }));
            },
            (thrown: unknown) => {
              const message = toCallError(thrown).error.message;
              store.set((s) => ({ ...s, error: message }));
            },
          );
        }
      });
    },
  };
}

/** The notifier the app uses. */
export type Notifier = ReturnType<typeof createNotifier>;
