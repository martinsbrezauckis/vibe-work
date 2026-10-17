/**
 * The Project tab's controller: per-session live hub (handbook pages, manifest, findings, visual
 * check badges), the page shown (facts + comments + the answers to its inputs), the viewer, the plan checklist's
 * current plan, and every action as a typed `pages` command. Components render this store and call these actions only.
 * Depends on: state/coreStore (`run`, `listen`), state/createStore, protocol types. Called by: register.tsx, the
 * Project tab components, pagesRoutes.ts, point.ts, the Projects overview (open a page), UI tests.
 * Invariants:
 * - Nothing here loads a page the core did not accept: refusals come back as errors and are shown as text.
 * - A `pagesChanged` event reloads what the store already shows for that session (a `null` session: all).
 * - `layout` calls are serialized and only the newest one is sent (a resize sends many).
 * - Page text (titles, labels, comments) is only ever rendered as plain text by the components.
 * -: a core started without Project Pages (a browser development host) answers `notInThisBuild`; that
 *   is remembered once (`unavailable`), shown inline, and the plan checklist stops asking. Concurrent refreshes
 *   coalesce into one request per session until its answer arrives; a changed supported plan gets one more read.
 */
import { toCallError } from "../../protocol/client/types";
import type {
  AnswerValue,
  PageAnswers,
  PagesRequest,
  PagesResult,
  PagesSettings,
  PickedSection,
  Rect,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";
import { pruneSessions } from "../../state/prune";
import { reuseAnswers } from "./inputs";

/** A session's hub answer. */
export type HubAnswer = Extract<PagesResult, { type: "hub" }>;
/** A session's page answer. */
export type PageAnswer = Extract<PagesResult, { type: "page" }>;
/** The plan checklist's answer. */
export type PlanAnswer = Extract<PagesResult, { type: "currentPlan" }>;

/** The tab's state. */
export interface PagesUi {
  hubs: Readonly<Record<string, HubAnswer>>;
  /** The page whose facts and comments are shown (the viewer's page). */
  pages: Readonly<Record<string, PageAnswer | null>>;
  /** Hub (`true`) or page view. */
  hubView: Readonly<Record<string, boolean>>;
  busy: Readonly<Record<string, boolean>>;
  picking: Readonly<Record<string, boolean>>;
  error: Readonly<Record<string, string | null>>;
  /** Local guidance after an action could not open a plan. */
  notice: Readonly<Record<string, string | null>>;
  /**: the shown page's inputs with the user's answers (`null`: the page has no inputs). */
  answers: Readonly<Record<string, PageAnswers | null>>;
  /**: the plan checklist's current plan, per session (absent: not asked yet). */
  plans: Readonly<Record<string, PlanAnswer>>;
  /**: a visual check of the shown page is running. */
  checking: Readonly<Record<string, boolean>>;
  /**: why this core has no Project Pages (it answered `notInThisBuild`), else `null`. */
  unavailable: string | null;
}

const initial: PagesUi = {
  hubs: {},
  pages: {},
  hubView: {},
  busy: {},
  picking: {},
  error: {},
  notice: {},
  answers: {},
  plans: {},
  checking: {},
  unavailable: null,
};

function put<T>(map: Readonly<Record<string, T>>, key: string, value: T): Record<string, T> {
  return { ...map, [key]: value };
}

/** Where the window shows the viewer: a session and its page area, or nothing. */
export type Shown = { sessionId: string; rect: Rect } | null;

function sameShown(a: Shown, b: Shown): boolean {
  if (a === null || b === null) return a === b;
  const [x, y] = [a.rect, b.rect];
  return (
    a.sessionId === b.sessionId &&
    x.x === y.x &&
    x.y === y.y &&
    x.width === y.width &&
    x.height === y.height
  );
}

/** Creates the controller for `core`. */
export function createPages(core: CoreController) {
  const store = createStore<PagesUi>(initial);
  const run = async (request: PagesRequest): Promise<PagesResult> => {
    const result = await core.run({ type: "pages", request });
    if (result.type !== "pages") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  type PerSession =
    | "hubs"
    | "pages"
    | "hubView"
    | "busy"
    | "picking"
    | "error"
    | "notice"
    | "answers"
    | "plans"
    | "checking";
  const set = (key: PerSession, sessionId: string, value: unknown) => {
    store.set((s) => ({ ...s, [key]: put(s[key] as Record<string, unknown>, sessionId, value) }));
  };
  const setAnswers = (sessionId: string, next: PageAnswers | null) => {
    const old = store.get().answers[sessionId] ?? null;
    const value = next === null ? null : reuseAnswers(old, next);
    if (value !== old) set("answers", sessionId, value);
  };
  const fail = (sessionId: string, thrown: unknown) => {
    const error = toCallError(thrown).error;
    if (error.serverCode === "notInThisBuild")
      store.set((s) => ({ ...s, unavailable: error.message }));
    set("error", sessionId, error.message);
  };
  /** Runs `request` with the busy flag; failures become the session's error text. */
  const act = async (sessionId: string, request: PagesRequest): Promise<PagesResult | null> => {
    set("busy", sessionId, true);
    try {
      const result = await run(request);
      set("error", sessionId, null);
      return result;
    } catch (thrown) {
      fail(sessionId, thrown);
      return null;
    } finally {
      set("busy", sessionId, false);
    }
  };

  let shown: Shown = null;
  let sent: Shown | undefined;
  let sending = false;
  const loadingPlans = new Set<string>();
  const queuedPlans = new Set<string>();
  const flushLayout = async (): Promise<void> => {
    if (sending) return;
    sending = true;
    try {
      while (sent === undefined || !sameShown(sent, shown)) {
        const target = shown;
        sent = target;
        try {
          await run(
            target === null
              ? { op: "layout" }
              : { op: "layout", sessionId: target.sessionId, rect: target.rect },
          );
        } catch (thrown) {
          if (target !== null) fail(target.sessionId, thrown);
        }
      }
    } finally {
      sending = false;
    }
  };

  const controller = {
    store,
    /** Reads the session's hub (and the page shown, when one is). */
    async load(sessionId: string): Promise<void> {
      try {
        const result = await run({ op: "hub", sessionId });
        if (result.type !== "hub") return;
        set("hubs", sessionId, result);
        const path = result.viewer?.path ?? null;
        if (path !== null && result.viewer !== null) await controller.loadPage(sessionId, path);
        else set("pages", sessionId, null);
      } catch (thrown) {
        fail(sessionId, thrown);
      }
    },
    /** Reads one page's facts and comments, and the answers to its inputs. */
    async loadPage(sessionId: string, path: string): Promise<void> {
      try {
        const result = await run({ op: "page", sessionId, path });
        if (result.type !== "page") return;
        set("pages", sessionId, result);
        if (result.facts.inputs.length === 0) {
          setAnswers(sessionId, null);
          return;
        }
        const answers = await run({ op: "answers", sessionId, path });
        setAnswers(sessionId, answers.type === "answers" ? (answers.pages[0] ?? null) : null);
      } catch (thrown) {
        fail(sessionId, thrown);
      }
    },
    /** Sets (or, with `null`, clears) the answer to one input of the shown page. */
    async answer(
      sessionId: string,
      path: string,
      input: string,
      value: AnswerValue | null,
    ): Promise<void> {
      const result = await act(
        sessionId,
        value === null
          ? { op: "clearAnswer", sessionId, path, input }
          : { op: "setAnswer", sessionId, path, input, value },
      );
      if (result?.type === "answers") setAnswers(sessionId, result.pages[0] ?? null);
    },
    /** Marks every answer of the page as put into the composer. */
    async markAnswersSent(sessionId: string, path: string): Promise<void> {
      const result = await act(sessionId, { op: "markAnswersSent", sessionId, path });
      if (result?.type === "answers") setAnswers(sessionId, result.pages[0] ?? null);
    },
    /** Runs the visual check of a page now; the hub's badges follow from `pagesChanged`. */
    async visualCheck(sessionId: string, path: string): Promise<PagesResult | null> {
      set("checking", sessionId, true);
      try {
        return await act(sessionId, { op: "visualCheck", sessionId, path });
      } finally {
        set("checking", sessionId, false);
      }
    },
    /** Opens a page of the session's project scrolled to section or step `anchor` (the plan checklist). */
    async openAt(sessionId: string, path: string, anchor: string): Promise<void> {
      set("hubView", sessionId, false);
      const result = await act(sessionId, { op: "open", sessionId, path, anchor });
      if (result !== null) await controller.load(sessionId);
    },
    /** Reads the plan checklist's current plan (the core then watches its file). */
    async loadPlan(sessionId: string): Promise<void> {
      if (store.get().unavailable !== null) return;
      if (loadingPlans.has(sessionId)) {
        // A change during a read may contain newer facts, so keep one follow-up for supported cores.
        queuedPlans.add(sessionId);
        return;
      }
      loadingPlans.add(sessionId);
      try {
        const result = await run({ op: "currentPlan", sessionId });
        if (result.type === "currentPlan") set("plans", sessionId, result);
      } catch (thrown) {
        fail(sessionId, thrown);
      } finally {
        loadingPlans.delete(sessionId);
        if (queuedPlans.delete(sessionId) && store.get().unavailable === null)
          void controller.loadPlan(sessionId);
      }
    },
    /** Every page with inputs (no `path`) or one page, with the user's answers (the Debug API route). */
    answersFor: (sessionId: string, path?: string) =>
      act(
        sessionId,
        path === undefined ? { op: "answers", sessionId } : { op: "answers", sessionId, path },
      ),
    /** Exports a page as one file: the text back, or (Debug API only) written to `destination`. */
    exportPage: (sessionId: string, path: string, destination?: string) =>
      act(
        sessionId,
        destination === undefined
          ? { op: "exportPage", sessionId, path }
          : { op: "exportPage", sessionId, path, destination },
      ),
    /** Tells the viewer the app's theme (it passes it to every page; Format 1.2 §6.4). */
    setTheme: (theme: "light" | "dark") => run({ op: "theme", theme }).catch(() => undefined),
    /** Shows the hub list (the viewer is hidden while it shows). */
    showHub(sessionId: string): void {
      set("hubView", sessionId, true);
      set("notice", sessionId, null);
    },
    /** Opens a page of the session's project in the viewer. */
    async open(sessionId: string, path: string): Promise<void> {
      set("hubView", sessionId, false);
      const result = await act(sessionId, { op: "open", sessionId, path });
      if (result !== null) await controller.load(sessionId);
    },
    /** Opens the current plan (the newest plan page). Returns false when there is none. */
    async openPlan(sessionId: string): Promise<boolean> {
      set("hubView", sessionId, false);
      const result = await act(sessionId, { op: "plan", sessionId });
      if (result?.type !== "page") {
        if (result !== null) {
          set("hubView", sessionId, true);
          set("notice", sessionId, "This project has no plan page yet.");
        }
        return false;
      }
      set("notice", sessionId, null);
      set("pages", sessionId, result);
      await controller.load(sessionId);
      return true;
    },
    back: (sessionId: string) => act(sessionId, { op: "back", sessionId }),
    forward: (sessionId: string) => act(sessionId, { op: "forward", sessionId }),
    reload: (sessionId: string) => act(sessionId, { op: "reload", sessionId }),
    /** Forgets the external link the page tried to open. */
    clearExternal: (sessionId: string) => act(sessionId, { op: "clearExternal", sessionId }),
    /**
     * Lets the user point at a section of the shown page; `null` when nothing was picked. With `section` the section
     * is named instead (a Debug API request has no pointer) and the answer is immediate.
     */
    async pickSection(sessionId: string, section?: string): Promise<PickedSection | null> {
      set("picking", sessionId, true);
      try {
        const result = await act(sessionId, {
          op: "pickSection",
          sessionId,
          ...(section === undefined ? {} : { section }),
        });
        return result?.type === "picked" ? result.picked : null;
      } finally {
        set("picking", sessionId, false);
      }
    },
    cancelPick: () => run({ op: "cancelPick" }).catch(() => undefined),
    /** Stores a comment; returns its id, or `null` when refused (the error is shown). */
    async addComment(
      sessionId: string,
      target: { path: string; section: string; text: string; quote?: string },
    ): Promise<string | null> {
      const result = await act(sessionId, { op: "addComment", sessionId, ...target });
      await controller.loadPage(sessionId, target.path);
      return result?.type === "comment" ? result.comment.id : null;
    },
    async deleteComment(sessionId: string, path: string, id: string): Promise<void> {
      await act(sessionId, { op: "deleteComment", sessionId, id });
      await controller.loadPage(sessionId, path);
    },
    /** Marks comments as put into the composer. */
    async markSent(sessionId: string, path: string, ids: string[]): Promise<void> {
      await act(sessionId, { op: "markSent", sessionId, ids });
      await controller.loadPage(sessionId, path);
    },
    async setSettings(sessionId: string, settings: PagesSettings): Promise<void> {
      await act(sessionId, { op: "setSettings", sessionId, settings });
      await controller.load(sessionId);
    },
    /** Tells the core where the viewer is shown now (only the newest value is sent). */
    layout(next: Shown): void {
      shown = next;
      void flushLayout();
    },
    /** Stops following core events. */
    stop: () => {
      stopListening();
    },
  };

  // a deleted session's entries go with it.
  core.onSessionsGone((ids) => {
    store.set((s) =>
      pruneSessions(
        s,
        [
          "hubs",
          "pages",
          "hubView",
          "busy",
          "picking",
          "error",
          "notice",
          "answers",
          "plans",
          "checking",
        ],
        ids,
      ),
    );
  });
  const stopListening = core.listen((event) => {
    // after `lagged` the core's events for this were missed: read again what is shown.
    if (event.type !== "pagesChanged" && event.type !== "lagged") return;
    const { hubs, plans } = store.get();
    const one = event.type === "pagesChanged" ? event.sessionId : null;
    const ids = one === null ? Object.keys(hubs) : [one];
    for (const id of ids) if (hubs[id] !== undefined) void controller.load(id);
    const planIds = one === null ? Object.keys(plans) : [one];
    for (const id of planIds) if (plans[id] !== undefined) void controller.loadPlan(id);
  });

  return controller;
}

/** The controller type. */
export type Pages = ReturnType<typeof createPages>;
