/**
 * The transcript's own UI state and actions: which groups and steps are open, which text
 * is shown in full, callback answers in flight and their outcome, which callbacks this window
 * answered itself, and the link waiting in the link dialog. Rows unmount when virtualised away, so
 * their open/closed state lives here, not in components. Thinking and its containing groups
 * remember explicit choices across restarts; unspecified choices follow the shared display preference.
 * Depends on: createStore, coreStore (`CoreController`), toastStore, protocol types. Called by: the
 * transcript views, the `ui/transcript` route, createApp.
 * Invariants:
 * - An answer goes through the core's `answerCallback` command only (the gate validates its shape and
 *   refuses a second answer); the card is never shown as answered before Vibe says so.
 * - A refusal because the callback was already answered (`callbackRefused`) is shown as "Already
 *   answered", any other failure as its message with the buttons enabled again.
 * - "Answered in this window" survives a reload (localStorage, capped), so the audit row keeps saying
 *   who answered; storage failures are ignored.
 * - A link is opened only after the user confirmed it in the dialog, and only through the client's
 *   `openLink` (the core validates again). Web sources open the same dialog; the app never
 *   fetches a source page itself.
 */
import type { JsonValue } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import type { Toasts } from "./toastStore";

/** Where an answer was sent from (`model/callbacks.ts` `AnsweredVia`). */
export type AnswerVia = "window" | "uiControl";

/** The outcome of an answer that did not go through. */
export interface AnswerProblem {
  kind: "refused" | "error";
  message: string;
}

/** The transcript UI state. */
export interface TranscriptUi {
  /** Disclosure choices by key; false remembers a manual close of a default-open thinking block. */
  open: Readonly<Record<string, boolean>>;
  /** Callback ids with an answer in flight. */
  sending: Readonly<Record<string, true>>;
  /** Answers that did not go through, by callback id. */
  problems: Readonly<Record<string, AnswerProblem>>;
  /** Callbacks this window answered, by callback id. */
  answered: Readonly<Record<string, AnswerVia>>;
  /** The link waiting for confirmation in the link dialog. */
  link: string | null;
  /** Session ids whose stored state is being loaded. */
  loading: Readonly<Record<string, true>>;
  /** The answer row whose web sources the side list shows, and the sources. */
  sources: { key: string; items: readonly SourceItem[] } | null;
}

/** One web source in the side list (`features/transcript/model/sources.ts` `Source`). */
export interface SourceItem {
  title: string;
  url: string;
  host: string | null;
}

const ANSWERED_KEY = "vw.answeredCallbacks";
const DISCLOSURE_KEY = "vw.thinkingDisclosure";
/** Most remembered answers (oldest dropped). */
const MAX_ANSWERED = 1000;
/** Most remembered thinking and containing-group disclosure choices. */
const MAX_DISCLOSURE = 1000;

function loadDisclosure(persist: boolean): Record<string, boolean> {
  if (!persist) return {};
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(DISCLOSURE_KEY) ?? "{}");
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
    const entries = Object.entries(parsed as Record<string, unknown>)
      .filter(
        ([key, value]) =>
          (key.startsWith("think:") || key.startsWith("steps:")) && typeof value === "boolean",
      )
      .slice(-MAX_DISCLOSURE);
    return Object.fromEntries(entries) as Record<string, boolean>;
  } catch {
    return {};
  }
}

function saveDisclosure(persist: boolean, open: Readonly<Record<string, boolean>>): void {
  if (!persist) return;
  try {
    const entries = Object.entries(open)
      .filter(([key]) => key.startsWith("think:") || key.startsWith("steps:"))
      .slice(-MAX_DISCLOSURE);
    localStorage.setItem(DISCLOSURE_KEY, JSON.stringify(Object.fromEntries(entries)));
  } catch {
    // The disclosure still survives row unmounts in this window.
  }
}

function loadAnswered(persist: boolean): Record<string, AnswerVia> {
  if (!persist) return {};
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(ANSWERED_KEY) ?? "{}");
    if (typeof parsed !== "object" || parsed === null) return {};
    const out: Record<string, AnswerVia> = {};
    for (const [id, via] of Object.entries(parsed as Record<string, unknown>)) {
      if (via === "window" || via === "uiControl") out[id] = via;
    }
    return out;
  } catch {
    return {};
  }
}

function saveAnswered(persist: boolean, answered: Record<string, AnswerVia>): void {
  if (!persist) return;
  try {
    const entries = Object.entries(answered).slice(-MAX_ANSWERED);
    localStorage.setItem(ANSWERED_KEY, JSON.stringify(Object.fromEntries(entries)));
  } catch {
    // Not remembered across a reload; the audit row then says "outside this window".
  }
}

function without<T>(map: Readonly<Record<string, T>>, key: string): Record<string, T> {
  const next = { ...map };
  Reflect.deleteProperty(next, key);
  return next;
}

/** Creates the transcript state and its actions. `core` is `null` without a connection. */
export function createTranscript(deps: {
  core: CoreController | null;
  toasts: Toasts;
  persist?: boolean;
}) {
  const { core, toasts } = deps;
  const persist = deps.persist ?? true;
  const store = createStore<TranscriptUi>({
    open: loadDisclosure(persist),
    sending: {},
    problems: {},
    answered: loadAnswered(persist),
    link: null,
    loading: {},
    sources: null,
  });

  return {
    store,

    /** Opens or closes `key` (`open` given: sets it; omitted: toggles). */
    toggle(key: string, open?: boolean): void {
      store.set((s) => {
        const next = open ?? s.open[key] !== true;
        return { ...s, open: { ...s.open, [key]: next } };
      });
      if (key.startsWith("think:") || key.startsWith("steps:"))
        saveDisclosure(persist, store.get().open);
    },

    /**
     * Sends an answer for one callback. Resolves to `true` when the core accepted it; the card shows
     * "Sending" meanwhile and the outcome afterwards. Never throws.
     */
    async answer(
      sessionId: string,
      callbackId: string,
      output: JsonValue,
      via: AnswerVia,
    ): Promise<boolean> {
      if (core === null) return false;
      if (store.get().sending[callbackId] === true) return false;
      store.set((s) => ({
        ...s,
        sending: { ...s.sending, [callbackId]: true },
        problems: without(s.problems, callbackId),
      }));
      try {
        // An answer through the `ui/transcript` route is the Debug API's: lowered explicitly.
        await core.run(
          { type: "answerCallback", sessionId, callbackId, output },
          via === "uiControl" ? { origin: "debug-api" } : undefined,
        );
        store.set((s) => ({ ...s, answered: { ...s.answered, [callbackId]: via } }));
        saveAnswered(persist, store.get().answered);
        return true;
      } catch (thrown) {
        const error = toCallError(thrown).error;
        const problem: AnswerProblem =
          error.code === "callbackRefused"
            ? { kind: "refused", message: "Already answered." }
            : { kind: "error", message: error.message };
        store.set((s) => ({ ...s, problems: { ...s.problems, [callbackId]: problem } }));
        return false;
      } finally {
        store.set((s) => ({ ...s, sending: without(s.sending, callbackId) }));
      }
    },

    /** Loads a session's stored history when the app only has its listing row. */
    async ensureLoaded(sessionId: string): Promise<void> {
      if (core === null || store.get().loading[sessionId] === true) return;
      const entry = core.store.get().sessions[sessionId];
      if (entry === undefined || entry.state !== null || entry.localOnly) return;
      store.set((s) => ({ ...s, loading: { ...s.loading, [sessionId]: true } }));
      try {
        await core.run({ type: "refreshSession", sessionId });
      } catch (thrown) {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: toCallError(thrown).error.message,
        });
      } finally {
        store.set((s) => ({ ...s, loading: without(s.loading, sessionId) }));
      }
    },

    /** Opens the side list of an answer's web sources (`null` closes it). */
    showSources(sources: TranscriptUi["sources"]): void {
      store.set((s) => ({ ...s, sources }));
    },

    /** Shows the link dialog for `url` (`null` closes it). */
    askLink(url: string | null): void {
      store.set((s) => ({ ...s, link: url }));
    },

    /** Opens a confirmed link in the system browser through the core; closes the dialog. */
    async openLink(url: string): Promise<boolean> {
      store.set((s) => ({ ...s, link: null }));
      if (core === null) return false;
      try {
        await core.client.openLink(url);
        return true;
      } catch (thrown) {
        toasts.push({
          kind: "error",
          bellKind: "failure",
          text: toCallError(thrown).error.message,
        });
        return false;
      }
    },
  };
}

/** The transcript state and actions of one app. */
export type Transcript = ReturnType<typeof createTranscript>;
