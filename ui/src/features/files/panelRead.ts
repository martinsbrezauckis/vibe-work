/**
 * The window's side of the agent's `panel_read`: on the core's `panelRead`
 * event it reads what that session's side panel shows (tabs, open editors with unsaved text, selection and cursor
 * line, the Files tab) and answers with `files panelAnswer`; meanwhile the session's editors show a short line that
 * Vibe is reading them.
 * Depends on: state/files (workspace), shellState (tab labels), liveEditors. Called by: register.tsx (the event),
 * git-browser/register (the Files tab's part), FileEditor (the reading line), tests.
 * Invariants: reads only; only the event's own session; no terminal output, browser page or form field is ever
 * part of the answer (those tabs are named only); unsaved text is sent only when it differs from the file on disk.
 */
import type { PanelEditor, PanelFiles, PanelView } from "../../protocol/core";
import { createStore } from "../../state/createStore";
import { TAB_META } from "../../state/shellState";
import type { App } from "../shell/AppContext";
import { liveEditor } from "./liveEditors";

/** How long the reading line stays after a read. */
const SHOWN_MS = 4000;
/** Most editors reported (the core keeps the same bound). */
const MAX_EDITORS = 20;

/** Sessions whose panel Vibe read just now: their editors show the reading line. */
export const panelReading = createStore<Readonly<Record<string, true>>>({});

let filesTab: ((sessionId: string) => PanelFiles | null) | null = null;

/** Installs (or removes) how the Files tab reports its folder, view and search text. */
export function setFilesTabSource(source: ((sessionId: string) => PanelFiles | null) | null): void {
  filesTab = source;
}

/** What the side panel of `sessionId` shows now. */
export function panelView(app: App, sessionId: string): PanelView {
  const shell = app.shell.store.get();
  const panel = shell.panels[sessionId];
  const files = app.files;
  const view = files?.workspace.of(sessionId);
  const docs = view?.documents ?? {};
  const name = (file: string) => {
    const doc = docs[file];
    const short = doc?.shown.display.split(/[\\/]/).pop() ?? file;
    return doc?.saved === undefined ? short : `${short} (${doc.savedName ?? "saved"} version)`;
  };
  const separate = shell.openFilesIn === "tabs";
  const tabs = (panel?.tabs ?? []).flatMap((kind) =>
    kind === "file"
      ? separate
        ? (view?.order ?? []).map(name)
        : ["File"]
      : [TAB_META[kind].label],
  );
  const shown = panel?.active ?? null;
  const active =
    shown === "file"
      ? separate && view?.active
        ? name(view.active)
        : "File"
      : shown === null
        ? null
        : TAB_META[shown].label;
  const split = view?.split ?? null;
  const editors = (view?.order ?? []).slice(0, MAX_EDITORS).flatMap((file): PanelEditor[] => {
    const doc = docs[file];
    if (doc === undefined || files === null) return [];
    const draft = files.document(sessionId, file)?.store.get().edits[sessionId];
    const handle = liveEditor(sessionId, file);
    const selected = handle?.selection() ?? null;
    const saved = doc.saved === undefined ? null : (doc.savedName ?? "saved");
    const unsaved = saved === null && draft !== undefined && draft.text !== draft.base.text;
    return [
      {
        path: doc.shown.display,
        focused: view?.active === file && (split === null || split[split.focus] === file),
        side:
          split === null
            ? null
            : split.left === file
              ? "left"
              : split.right === file
                ? "right"
                : null,
        savedVersion: saved,
        unsaved,
        unsavedText: unsaved && draft.text !== draft.disk.text ? draft.text : null,
        selection:
          selected === null
            ? null
            : { startLine: selected.start, endLine: selected.end, text: selected.text },
        cursorLine: handle?.getLine() ?? doc.shown.line,
        changedLines: [...(draft?.marks ?? [])],
      },
    ];
  });
  return {
    open: panel?.open ?? false,
    tabs,
    active,
    editors,
    files:
      panel?.tabs.includes("git") === true && shell.selected === sessionId
        ? (filesTab?.(sessionId) ?? null)
        : null,
  };
}

/** Answers the core's `panelRead` events; returns the function that stops. */
export function registerPanelRead(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const stop = core.listen((event) => {
    if (event.type !== "panelRead") return;
    const id = event.sessionId;
    panelReading.set((s) => ({ ...s, [id]: true }));
    clearTimeout(timers.get(id));
    timers.set(
      id,
      setTimeout(() => {
        timers.delete(id);
        panelReading.set((s) => Object.fromEntries(Object.entries(s).filter(([k]) => k !== id)));
      }, SHOWN_MS),
    );
    void core
      .run({
        type: "files",
        request: {
          op: "panelAnswer",
          sessionId: id,
          requestId: event.requestId,
          panel: panelView(app, id),
        },
      })
      .catch(() => {
        // The tool tells the agent that no window answered.
      });
  });
  return () => {
    stop();
    for (const timer of timers.values()) clearTimeout(timer);
    panelReading.set({});
  };
}
