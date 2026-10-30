/**
 * The `ui/editors` UI-control route (listed by the Rust host in `crates/vw-host/src/host.rs` FILES_ROUTES): the
 * editor workspace beside the chat, every action its controls and keys have, through the same functions a click
 * uses. The shown document's text and its editor stay on `ui/files`.
 * - `state` (default): the open files as the tabs and the +N list show them (name, path, marks, saved side,
 *   visible or behind +N), the focused one, the editor pair (left, right, focus, compare), the divider ratio, the
 *   folded editors, the explanation shown above the editors, the unsaved-changes dialog, the chat/sessions slot
 *   layout and the Open files in choice;
 * - `activate {file}` (a tab click or a +N row), `openToSide {file?}` (the split button, Ctrl \ or Ctrl Enter),
 *   `restore`, `focus {side}`, `ratio {ratio}` (the divider), `fold {side, folded?}` (an editor's fold button);
 * - `compareSaved` (Compare with saved version: the last commit's text on the left, read-only), and
 *   `closeSavedVersion` (Close saved version / Close comparison);
 * - `close {file?}` (the tab's X or Ctrl W: an unsaved file opens the dialog instead) and
 *   `closeDialog {choice: cancel|discard|save}` (the dialog's Cancel, Discard and Save and close);
 * - `closeSavedTabs` (the +N list's Close saved tabs);
 * - `chat {folded}` / `sessions {folded}` (the fold buttons and the strips' unfold buttons), `swap` (Ctrl B);
 * - `openFilesIn {mode: tabs|single}` (Settings > Appearance > Open files in);
 * - `previewToSide {file?}`: Preview to the side for an HTML or Markdown file (its code left, its preview right);
 * - `diskNotice {choice: gotIt|undoMerge|keepYours|takeVibes|showChanges|compare, file?}`: the buttons of the notice
 *   after a file changed on disk; `state.notices` lists each open file's notice (sentence, lines, marks,
 *   choices).
 * Depends on: state/files (workspace), closeDecision, openFiles, panelLayout, routeInput. Called by: register.tsx.
 * Invariants: bodies are validated (`invalidParams` names the field); the session defaults to the one the File
 * tab shows (the selected session, else the chosen folder's workspace); a file must be one of that session's open
 * documents (`notFound`); Save and close never overwrites a newer file on disk (see closeDecision).
 */
import type { Files } from "../../state/files";
import { computeLayout } from "../../state/panelLayout";
import { panelOf } from "../../state/shellState";
import { bodyObject, boolField, isStateRead, oneOf, textField } from "../../state/routeInput";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";
import { decideClose } from "./closeDecision";
import { applyChoice, choicesOf, noticeText } from "./model/diskNotice";
import { openFileRows } from "./openFiles";
import { liveCodeActions } from "./liveEditors";
import { SEARCH_COMMANDS } from "../editor/searchCommands";

const SIDES = ["left", "right"] as const;

/** The session whose File tab the window shows (the same key FileTab uses). */
function shownSession(app: App, given: unknown): string {
  if (typeof given === "string" && given !== "") return given;
  const s = app.shell.store.get();
  const id = s.selected ?? (s.newFolder ? `files-workspace:${s.newFolder}` : null);
  if (id === null) throw new UiControlError("invalidParams", "no session is selected");
  return id;
}

/** Each open file's notice after a change on disk, in the words the window shows. */
function notices(files: Files, sessionId: string) {
  const view = files.workspace.of(sessionId);
  return view.order.flatMap((file) => {
    const doc = view.documents[file];
    const draft = files.document(sessionId, file)?.store.get().edits[sessionId];
    if (doc === undefined || draft?.notice == null) return [];
    const name = doc.shown.display.split(/[\\/]/).pop() ?? file;
    return [
      {
        file,
        kind: draft.notice.kind,
        text: noticeText(draft.notice, name, doc.changed),
        lines: draft.notice.lines,
        marks: draft.marks,
        choices: [
          ...choicesOf(draft.notice),
          ...(draft.notice.kind === "overlap" ? ["compare"] : []),
          ...(draft.notice.kind === "reloaded" ? ["showChanges"] : []),
        ],
      },
    ];
  });
}

/** The route's answer: what the editor workspace and the shared slot show now. */
function state(app: App, files: Files, sessionId: string): unknown {
  const view = files.workspace.of(sessionId);
  const shell = app.shell.store.get();
  const layout = computeLayout(shell, window.innerWidth);
  const closing = view.closing;
  return {
    sessionId,
    openFilesIn: shell.openFilesIn,
    panel: { open: panelOf(shell).open, active: panelOf(shell).active },
    files: openFileRows(files, sessionId),
    hidden: view.order.length - view.visible.length,
    active: view.active,
    code:
      view.active === null ? null : (liveCodeActions(sessionId, view.active)?.codeState() ?? null),
    split: view.split,
    ratio: view.ratio,
    folded: view.folded,
    problem: view.problem,
    notices: notices(files, sessionId),
    closeDialog:
      closing === null
        ? null
        : {
            file: closing,
            title: `Save changes to ${view.documents[closing]?.shown.display ?? "the file"}?`,
            problem: view.closeProblem,
          },
    layout: {
      editorSplit: layout.editorSplit,
      slot: layout.slot,
      chatFolded: layout.chatFolded,
      listVisible: layout.listVisible,
      slotWidth: layout.slotWidth,
    },
  };
}

/** An open document of the session, named by `file` (the focused one when `optional` and absent). */
function openDocument(
  files: Files,
  sessionId: string,
  input: Record<string, unknown>,
  optional: boolean,
): string {
  const view = files.workspace.of(sessionId);
  const file = optional && input["file"] === undefined ? view.active : textField(input, "file");
  if (file === null) throw new UiControlError("invalidParams", "no file is open in this session");
  if (view.documents[file] === undefined)
    throw new UiControlError("notFound", "that file is not open in this session (see state.files)");
  return file;
}

/** Folds or unfolds the session list the way its fold button and the folded strip's button do. */
function sessions(app: App, folded: boolean): void {
  const shell = app.shell;
  const layout = computeLayout(shell.store.get(), window.innerWidth);
  if (folded) {
    if (layout.editorSplit) shell.actions.unfoldChat();
    else shell.actions.toggleList(true);
  } else if (layout.editorSplit) {
    if (layout.slot !== "list") shell.actions.swapEditorSlot();
  } else shell.actions.showList();
}

/** One `ui/editors` request. */
async function route(app: App, files: Files, body: unknown): Promise<unknown> {
  const input = bodyObject(body);
  const sessionId = shownSession(app, input["sessionId"]);
  const ws = files.workspace;
  const action = input["action"] ?? "state";
  switch (action) {
    case "find":
    case "goToLine":
    case "findNext":
    case "findPrevious":
    case "selectMatches":
    case "replaceNext":
    case "replaceAll":
    case "closeFind":
    case "foldCode": {
      const file = openDocument(files, sessionId, input, true);
      const editor = liveCodeActions(sessionId, file);
      if (editor === undefined)
        throw new UiControlError("notFound", "that file has no mounted code editor");
      if (action === "find") {
        const query = input["query"];
        if (typeof query !== "string")
          throw new UiControlError("invalidParams", "query must be a string");
        const replace = input["replace"];
        if (replace !== undefined && typeof replace !== "string")
          throw new UiControlError("invalidParams", "replace must be a string");
        editor.search({
          query,
          ...(replace === undefined ? {} : { replace }),
          caseSensitive:
            input["caseSensitive"] === undefined ? false : boolField(input, "caseSensitive"),
          regexp: input["regexp"] === undefined ? false : boolField(input, "regexp"),
          wholeWord: input["wholeWord"] === undefined ? false : boolField(input, "wholeWord"),
        });
      } else if (action === "foldCode" || action === "goToLine") {
        const line = input["line"];
        if (typeof line !== "number" || !Number.isInteger(line) || line < 1)
          throw new UiControlError("invalidParams", "line must be a positive integer");
        if (action === "goToLine") editor.goToLine(line);
        else editor.foldCode(line, boolField(input, "folded"));
      } else {
        const command =
          action === "findNext"
            ? "next"
            : action === "findPrevious"
              ? "previous"
              : action === "selectMatches"
                ? "selectAll"
                : action === "closeFind"
                  ? "close"
                  : action;
        const known = SEARCH_COMMANDS.find((item) => item === command);
        if (known !== undefined) editor.searchCommand(known);
      }
      return { ...(state(app, files, sessionId) as object), code: editor.codeState() };
    }
    case "state":
      break;
    case "activate":
      ws.activate(sessionId, openDocument(files, sessionId, input, false));
      app.shell.actions.openTab("file");
      break;
    case "openToSide": {
      const file = openDocument(files, sessionId, input, true);
      const shown = ws.of(sessionId).documents[file]?.shown;
      if (shown === undefined || ws.of(sessionId).documents[file]?.saved !== undefined)
        throw new UiControlError("invalidParams", "a saved version cannot be opened to the side");
      await ws.openToSide(sessionId, shown);
      break;
    }
    case "previewToSide": {
      const file = openDocument(files, sessionId, input, true);
      const doc = ws.of(sessionId).documents[file];
      const kind = files.document(sessionId, file)?.store.get().tabs[sessionId]?.content?.kind;
      if (
        doc === undefined ||
        doc.saved !== undefined ||
        doc.preview ||
        (kind !== "markdown" && kind !== "html")
      )
        throw new UiControlError(
          "invalidParams",
          "only an HTML or Markdown file has a preview to the side",
        );
      ws.previewToSide(sessionId, doc.shown);
      break;
    }
    case "restore":
      ws.restore(sessionId);
      break;
    case "focus":
      if (ws.of(sessionId).split === null)
        throw new UiControlError("invalidParams", "no editor pair is open");
      ws.focus(sessionId, oneOf(input, "side", SIDES));
      break;
    case "ratio": {
      const ratio = input["ratio"];
      if (typeof ratio !== "number" || !Number.isFinite(ratio))
        throw new UiControlError("invalidParams", "ratio must be a number (0.25 to 0.75)");
      ws.ratio(sessionId, ratio);
      break;
    }
    case "fold": {
      const side = oneOf(input, "side", SIDES);
      const want = input["folded"] === undefined ? null : boolField(input, "folded");
      if (want === null || ws.of(sessionId).folded.includes(side) !== want)
        ws.fold(sessionId, side);
      break;
    }
    case "compareSaved": {
      const active = ws.of(sessionId).active;
      const doc = active === null ? undefined : ws.of(sessionId).documents[active];
      const reader = doc ? files.document(sessionId, doc.shown.file) : null;
      if (reader?.store.get().tabs[sessionId]?.content?.text == null)
        throw new UiControlError("invalidParams", "the focused file has no text to compare");
      await ws.compareSaved(sessionId);
      break;
    }
    case "closeSavedVersion": {
      const pair = ws.of(sessionId).split;
      if (pair?.compare !== true)
        throw new UiControlError("invalidParams", "no comparison is open");
      ws.close(sessionId, pair.left);
      break;
    }
    case "close": {
      const file = openDocument(files, sessionId, input, true);
      ws.requestClose(sessionId, file);
      return {
        closed: ws.of(sessionId).documents[file] === undefined,
        ...(state(app, files, sessionId) as object),
      };
    }
    case "closeDialog": {
      const file = ws.of(sessionId).closing;
      if (file === null)
        throw new UiControlError("invalidParams", "no unsaved-changes dialog is open");
      const choice = oneOf(input, "choice", ["cancel", "discard", "save"] as const);
      const outcome = await decideClose(app, files, sessionId, file, choice, "debug-api");
      return { outcome, ...(state(app, files, sessionId) as object) };
    }
    case "closeSavedTabs":
      ws.closeSaved(sessionId);
      break;
    case "chat":
      if (boolField(input, "folded")) {
        if (!computeLayout(app.shell.store.get(), window.innerWidth).panelOpen)
          throw new UiControlError("invalidParams", "the chat folds beside an open side panel");
        app.shell.actions.foldChat();
      } else app.shell.actions.unfoldChat();
      break;
    case "sessions":
      sessions(app, boolField(input, "folded"));
      break;
    case "swap": {
      const toggle = app.commands.list().find((c) => c.id === "toggle-list");
      if (toggle === undefined)
        throw new UiControlError("serverUnavailable", "Ctrl B is not registered");
      toggle.run();
      break;
    }
    case "openFilesIn":
      app.shell.actions.setOpenFilesIn(oneOf(input, "mode", ["tabs", "single"] as const));
      break;
    case "diskNotice": {
      const file = openDocument(files, sessionId, input, true);
      const reader = files.document(sessionId, file);
      const draft = reader?.store.get().edits[sessionId];
      const choice = oneOf(input, "choice", [
        "gotIt",
        "undoMerge",
        "keepYours",
        "takeVibes",
        "showChanges",
        "compare",
      ] as const);
      const offered = draft?.notice == null ? [] : choicesOf(draft.notice);
      const window =
        (choice === "showChanges" && draft?.notice?.kind === "reloaded") ||
        (choice === "compare" && draft?.notice?.kind === "overlap");
      if (reader === null || draft === undefined || (!window && !offered.some((c) => c === choice)))
        throw new UiControlError("invalidParams", `that file's notice does not offer ${choice}`);
      if (choice === "compare" && draft.disk.text !== null) {
        ws.activate(sessionId, file);
        ws.compare(sessionId, draft.disk.text, "disk");
      } else if (choice === "showChanges") {
        // The window scrolls its editor; the route moves the editor's line to the first changed one.
        ws.activate(sessionId, file);
        const first = draft.notice?.lines[0] ?? 1;
        const shown = ws.of(sessionId).documents[file]?.shown;
        if (shown) await files.debugApi.show(sessionId, { ...shown, line: first });
      } else if (choice !== "compare") reader.edit.apply(sessionId, (d) => applyChoice(d, choice));
      break;
    }
    default:
      throw new UiControlError("invalidParams", `unknown action ${JSON.stringify(action)}`);
  }
  return state(app, files, sessionId);
}

/** Registers the route; returns the function that removes it. */
export function registerEditorsRoute(app: App, files: Files): () => void {
  return app.uiControl.register("editors", (body) => route(app, files, body), isStateRead);
}
