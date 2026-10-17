/**
 * The agent browser's controller in the Browser tab: the agent strip and action log per session
 * (`browser state`, live through `browserChanged`), Stop / Resume, and the user's annotations of the app preview
 * (pick or draw, note, collect, send as one batch, then check again after the agent's turn). Components render this
 * store and call these actions only.
 * Depends on: state/coreStore (`run`, `listen`, the session store), state/createStore, pages/point (`insertChip`),
 * browserModel.ts, protocol types. Called by: register.tsx, AgentStrip, ActionLog, Annotations, browserRoutes.ts,
 * UI tests.
 * Invariants:
 * - Annotations go to the composer as context chips plus pictures (data, never a command); nothing is
 *   sent to the agent by itself: the user sends the message.
 * - A note created or edited by a Debug API request keeps `by: "debug-api"`; when sent (by anyone), its chip and
 *   crop carry the Debug API's mark; the window's own notes sent from the window carry the window's.
 * - After a batch was sent, the session's next finished turn triggers one recheck (reload + per-annotation state).
 * - `onAgentStart` fires once each time the agent starts acting in a session (the tab opens for it).
 */
import { toCallError } from "../../protocol/client/types";
import type {
  BrowserRequest,
  BrowserResult,
  BrowserState,
  DeviceWidth,
  PageTab,
  PickTarget,
} from "../../protocol/core";
import type { CoreController } from "../../state/coreStore";
import { createStore } from "../../state/createStore";
import { runningTurnId } from "../composer/model/session";
import {
  type NoteItem,
  type NoteOrigin,
  annotationChip,
  base64Bytes,
  sendMark,
} from "./browserModel";

/** The Browser tab's agent-browser state. */
export interface BrowserUi {
  states: Readonly<Record<string, BrowserState>>;
  tabs: Readonly<Record<string, readonly PageTab[]>>;
  notes: Readonly<Record<string, readonly NoteItem[]>>;
  /** An annotation pick is running in this session. */
  picking: Readonly<Record<string, "element" | "box" | undefined>>;
  error: Readonly<Record<string, string | null>>;
}

const initial: BrowserUi = { states: {}, tabs: {}, notes: {}, picking: {}, error: {} };

function put<T>(map: Readonly<Record<string, T>>, key: string, value: T): Record<string, T> {
  return { ...map, [key]: value };
}

/** Who sends: the window's own controls, or a Debug API client (`ui/browser`): chips and pictures carry it. */
export type SendOrigin = NoteOrigin;

/** Opaque native choice; the chosen page value never enters this object or the window. */
export interface VaultChoice {
  handle: string;
  suggestedName: string;
  page: string;
  element: string | null;
  valueLength: number;
  tabId?: string;
}

/** Exact page review supplied by the core before a user tab can be handed to the agent. */
export interface HandoffReview {
  tabId: string;
  url: string | null;
  site: string | null;
  fingerprint: string;
}

/** What sending needs from the app: put a chip / a picture into a session's composer. */
export interface ComposerSink {
  chip: (
    sessionId: string,
    chip: { label: string; source: string; text: string },
    origin: SendOrigin,
  ) => boolean;
  picture: (
    sessionId: string,
    name: string,
    data: string,
    bytes: number,
    origin: SendOrigin,
  ) => void;
}

/** Creates the controller for `core`. */
export function createBrowser(
  core: CoreController,
  sink: ComposerSink,
  onAgentStart: (sessionId: string) => void,
) {
  const store = createStore<BrowserUi>(initial);
  let counter = 0;
  const tabsRead = new Map<string, number>();

  const run = async (request: BrowserRequest): Promise<BrowserResult> => {
    const result = await core.run({ type: "browser", request });
    if (result.type !== "browser") throw new Error(`unexpected ${result.type} result`);
    return result.result;
  };
  const setError = (sessionId: string, error: string | null) => {
    store.set((s) => ({ ...s, error: put(s.error, sessionId, error) }));
  };
  const setNotes = (
    sessionId: string,
    update: (notes: readonly NoteItem[]) => readonly NoteItem[],
  ) => {
    store.set((s) => ({ ...s, notes: put(s.notes, sessionId, update(s.notes[sessionId] ?? [])) }));
  };
  const setState = (state: BrowserState) => {
    const before = store.get().states[state.sessionId];
    store.set((s) => ({ ...s, states: put(s.states, state.sessionId, state) }));
    if (state.inFlight && before?.inFlight !== true) onAgentStart(state.sessionId);
  };
  const setTabs = (sessionId: string, tabs: readonly PageTab[]) => {
    tabsRead.set(sessionId, (tabsRead.get(sessionId) ?? 0) + 1);
    store.set((s) => ({ ...s, tabs: put(s.tabs, sessionId, tabs) }));
    setError(sessionId, null);
  };

  const controller = {
    store,
    /** Reads page tabs. The selected tab and the agent's current web tab are separate. */
    async loadTabs(sessionId: string): Promise<void> {
      const read = (tabsRead.get(sessionId) ?? 0) + 1;
      tabsRead.set(sessionId, read);
      try {
        const result = await run({ op: "tabs", sessionId });
        if (result.type === "tabs" && tabsRead.get(sessionId) === read)
          setTabs(sessionId, result.tabs);
      } catch (thrown) {
        if (tabsRead.get(sessionId) === read)
          setError(sessionId, toCallError(thrown).error.message);
      }
    },
    async openTab(sessionId: string, url?: string): Promise<void> {
      try {
        const result = await run({ op: "openTab", sessionId, url: url ?? null });
        if (result.type === "tabs") setTabs(sessionId, result.tabs);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    async switchTab(sessionId: string, tabId: string): Promise<void> {
      try {
        const result = await run({ op: "switchTab", sessionId, tabId });
        if (result.type === "tabs") setTabs(sessionId, result.tabs);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    async closeTab(sessionId: string, tabId: string): Promise<void> {
      try {
        const result = await run({ op: "closeTab", sessionId, tabId });
        if (result.type === "tabs") setTabs(sessionId, result.tabs);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    async reorderTabs(sessionId: string, tabIds: string[]): Promise<void> {
      try {
        const result = await run({ op: "reorderTabs", sessionId, tabIds });
        if (result.type === "tabs") setTabs(sessionId, result.tabs);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    async reviewHandoff(sessionId: string, tabId: string): Promise<HandoffReview | null> {
      try {
        const result = await run({ op: "reviewHandoff", sessionId, tabId });
        if (result.type !== "handoffReview") throw new Error("No tab review was returned.");
        const review = result.review;
        if (
          review === null ||
          typeof review !== "object" ||
          Array.isArray(review) ||
          typeof review.tabId !== "string" ||
          typeof review.fingerprint !== "string"
        ) {
          throw new Error("The tab review is incomplete.");
        }
        setError(sessionId, null);
        return {
          tabId: review.tabId,
          fingerprint: review.fingerprint,
          url: typeof review.url === "string" ? review.url : null,
          site: typeof review.site === "string" ? review.site : null,
        };
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
        return null;
      }
    },
    async handoffTab(sessionId: string, review: HandoffReview): Promise<boolean> {
      try {
        const result = await run({
          op: "handoffTab",
          sessionId,
          tabId: review.tabId,
          fingerprint: review.fingerprint,
          confirmed: true,
        });
        if (result.type !== "tabs") throw new Error("The tab handoff was not confirmed.");
        setTabs(sessionId, result.tabs);
        return true;
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
        return false;
      }
    },
    async takeBackTab(sessionId: string, tabId: string): Promise<void> {
      try {
        const result = await run({ op: "takeBackTab", sessionId, tabId });
        if (result.type === "tabs") setTabs(sessionId, result.tabs);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    /** Changes one owned web page's device width and refreshes the tab snapshot. */
    async setWebWidth(sessionId: string, tabId: string, width: DeviceWidth): Promise<void> {
      try {
        const result = await core.run({
          type: "preview",
          request: { op: "setWidth", sessionId, tabId, width },
        });
        if (result.type !== "preview" || result.result.type !== "preview")
          throw new Error("The page width was not confirmed.");
        await controller.loadTabs(sessionId);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    /** User navigation targets the visible web tab, never the pinned App preview. */
    async webAction(sessionId: string, tabId: string, action: string, url?: string): Promise<void> {
      try {
        await run({ op: "act", sessionId, tabId, args: { action, ...(url ? { url } : {}) } });
        await controller.loadTabs(sessionId);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    /** Reads a session's strip and log. */
    async load(sessionId: string): Promise<void> {
      try {
        const result = await run({ op: "state", sessionId });
        if (result.type === "state") setState(result.state);
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    /** The user's Stop: cancels the agent's browser action and refuses more until Resume or the next message. */
    async stopAgent(sessionId: string): Promise<void> {
      const result = await run({ op: "stop", sessionId });
      if (result.type === "state") setState(result.state);
    },
    /** Lets the agent use the browser again. */
    async resumeAgent(sessionId: string): Promise<void> {
      const result = await run({ op: "resume", sessionId });
      if (result.type === "state") setState(result.state);
    },
    /** Asks the native page for a user-selected value; receives metadata and an opaque handle only. */
    async chooseSecret(
      sessionId: string,
      mode: "selection" | "element" | "contextSelection",
      tabId?: string,
    ): Promise<VaultChoice> {
      const result = await run({
        op: "chooseSecret",
        sessionId,
        mode,
        ...(tabId ? { tabId } : {}),
      });
      if (result.type !== "secretChoice") throw new Error("The page did not return a choice.");
      return { ...result.choice, ...(tabId ? { tabId } : {}) };
    },
    /** Saves the previously chosen native value under the user's name. */
    async saveSecret(
      sessionId: string,
      handle: string,
      name: string,
      tabId?: string,
    ): Promise<void> {
      const result = await run({
        op: "saveSecret",
        sessionId,
        handle,
        name,
        ...(tabId ? { tabId } : {}),
      });
      if (result.type !== "secretStored") throw new Error("The vault did not confirm the save.");
    },
    /** Releases a chosen native value when its name prompt is dismissed or leaves the screen. */
    async cancelSecret(sessionId: string, handle: string, tabId?: string): Promise<void> {
      const result = await run({
        op: "cancelSecret",
        sessionId,
        handle,
        ...(tabId ? { tabId } : {}),
      });
      if (result.type !== "done") throw new Error("The page did not cancel the choice.");
    },
    /**
     * Lets the user point at an element (or draw a box) in the app preview; adds it as a draft annotation. With
     * `target` (a Debug API request, which has no pointer) the element or area is named instead and the answer is
     * immediate.
     */
    async annotate(
      sessionId: string,
      mode: "element" | "box",
      origin: NoteOrigin = "ui",
      target?: PickTarget,
    ): Promise<NoteItem | null> {
      store.set((s) => ({ ...s, picking: put(s.picking, sessionId, mode) }));
      try {
        const result = await run({
          op: "annotate",
          sessionId,
          mode,
          ...(target === undefined ? {} : { target }),
        });
        if (result.type !== "annotation") return null;
        counter += 1;
        const item: NoteItem = {
          id: `note-${String(counter)}`,
          annotation: result.annotation,
          note: "",
          status: "draft",
          by: origin,
        };
        setNotes(sessionId, (notes) => [...notes, item]);
        setError(sessionId, null);
        return item;
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
        return null;
      } finally {
        store.set((s) => ({ ...s, picking: put(s.picking, sessionId, undefined) }));
      }
    },
    /** Cancels a running pick (removes the overlay in the page). */
    async cancelPick(sessionId: string): Promise<void> {
      await run({ op: "cancelAnnotate", sessionId }).catch(() => undefined);
    },
    /** Sets a draft annotation's note (an edit by the Debug API marks the note as its own for good). */
    setNote(sessionId: string, id: string, note: string, origin: NoteOrigin = "ui"): void {
      setNotes(sessionId, (notes) =>
        notes.map((n) =>
          n.id === id ? { ...n, note, by: origin === "debug-api" ? "debug-api" : n.by } : n,
        ),
      );
    },
    /** Removes an annotation. */
    remove(sessionId: string, id: string): void {
      setNotes(sessionId, (notes) => notes.filter((n) => n.id !== id));
    },
    /** Puts every draft annotation into the composer (a chip each, and its crop as a picture). Returns how many. */
    send(sessionId: string, origin: SendOrigin = "ui"): number {
      const drafts = (store.get().notes[sessionId] ?? []).filter((n) => n.status === "draft");
      let sent = 0;
      for (const item of drafts) {
        const mark = sendMark(item, origin);
        if (!sink.chip(sessionId, annotationChip(item), mark)) break;
        const crop = item.annotation.cropPngBase64;
        if (crop !== null) sink.picture(sessionId, `${item.id}.png`, crop, base64Bytes(crop), mark);
        sent += 1;
      }
      const ids = new Set(drafts.slice(0, sent).map((n) => n.id));
      setNotes(sessionId, (notes) =>
        notes.map((n) => (ids.has(n.id) ? { ...n, status: "sent" as const } : n)),
      );
      if (sent > 0) waiting.add(sessionId);
      return sent;
    },
    /** Reloads the preview and says per sent annotation whether its element changed. */
    async recheck(sessionId: string, reload = true): Promise<void> {
      const checked = (store.get().notes[sessionId] ?? []).filter((n) => n.status !== "draft");
      if (checked.length === 0) return;
      try {
        const result = await run({
          op: "checkAnnotations",
          sessionId,
          reload,
          items: checked.map((n) => ({
            ...(n.annotation.reference !== null ? { reference: n.annotation.reference } : {}),
            text: n.annotation.text,
          })),
        });
        if (result.type !== "annotationStates") return;
        const states = new Map(checked.map((n, i) => [n.id, result.states[i]]));
        setNotes(sessionId, (notes) =>
          notes.map((n) => {
            const state = states.get(n.id);
            return state === "changed" || state === "unchanged" || state === "missing"
              ? { ...n, status: state }
              : n;
          }),
        );
      } catch (thrown) {
        setError(sessionId, toCallError(thrown).error.message);
      }
    },
    /** Stops following events. */
    dispose: () => {
      stopListening();
      stopTurns();
    },
  };

  // Sessions with a sent batch: their next finished turn triggers one recheck.
  const waiting = new Set<string>();
  const running = new Map<string, boolean>();
  const stopTurns = core.store.subscribe(() => {
    const sessions = core.store.get().sessions;
    for (const id of waiting) {
      const e = sessions[id];
      const now = runningTurnId(e?.state ?? null, e?.session) !== null;
      const was = running.get(id) ?? false;
      running.set(id, now);
      if (was && !now) {
        waiting.delete(id);
        void controller.recheck(id);
      }
    }
  });

  const stopListening = core.listen((event) => {
    if (event.type !== "browserChanged" && event.type !== "previewChanged") return;
    const states = store.get().states;
    const ids = event.sessionId === null ? Object.keys(states) : [event.sessionId];
    for (const id of ids) {
      if (event.type === "browserChanged") void controller.load(id);
      void controller.loadTabs(id);
    }
  });

  return controller;
}

/** The controller type. */
export type Browser = ReturnType<typeof createBrowser>;
