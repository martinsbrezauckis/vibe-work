/**
 * The right-click menu of a compare (Compare with saved version, a commit, or disk): the read-only left side
 * offers Point Vibe at the saved version, Copy and Close the saved version; the editable right side is code
 * (the code menu). Opens by right click, the menu key or Shift F10 inside either side.
 * Depends on: CodeMirror views of the MergeView, files/codeMenu, RowMenu, AppContext.
 * Called by: SavedComparison.
 */
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactElement,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import type { MergeView } from "@codemirror/merge";
import { gotoLine, openSearchPanel } from "@codemirror/search";
import type { EditorView } from "@codemirror/view";
import { RowMenu, isMenuKey, useRowMenu } from "../../components/RowMenu";
import type { FileReader } from "../../state/files";
import type { Files } from "../../state/files";
import { type CodeSurface, codeMenu } from "../files/codeMenu";
import { savedWords } from "../files/openFiles";
import type { App } from "../shell/AppContext";

type Built = ReturnType<typeof codeMenu>;

/** A CodeMirror view as a code surface. */
function viewSurface(view: EditorView): CodeSurface {
  const readOnly = view.state.readOnly;
  return {
    selection: () => {
      const { from, to, empty } = view.state.selection.main;
      if (empty) return null;
      return {
        start: view.state.doc.lineAt(from).number,
        end: view.state.doc.lineAt(Math.max(from, to - 1)).number,
        text: view.state.sliceDoc(from, to),
      };
    },
    line: () => view.state.doc.lineAt(view.state.selection.main.head).number,
    lineText: (line) => view.state.doc.line(Math.max(1, Math.min(line, view.state.doc.lines))).text,
    replace: (text) => {
      if (view.state.readOnly) return false;
      view.dispatch(view.state.replaceSelection(text));
      view.focus();
      return true;
    },
    find: () => {
      openSearchPanel(view);
      openSearchPanel(view);
    },
    goToLine: () => {
      gotoLine(view);
    },
    readOnly,
  };
}

/** The compare's menu; spread `triggers` on the compare's host element. */
export function useCompareMenu(
  app: App,
  files: Files,
  reader: FileReader,
  sessionId: string,
  engine: RefObject<MergeView | null>,
): {
  triggers: {
    onContextMenu: (event: ReactMouseEvent) => void;
    onKeyDown: (event: ReactKeyboardEvent) => void;
  };
  element: ReactElement | null;
} {
  const menu = useRowMenu<Built>();
  const build = (path: readonly EventTarget[]): Built | null => {
    const merge = engine.current;
    const pair = files.workspace.of(sessionId).split;
    const doc = pair ? files.workspace.of(sessionId).documents[pair.left] : undefined;
    const draft = reader.store.get().edits[sessionId];
    if (merge === null || !pair || doc === undefined) return null;
    const left = path.some((t) => t instanceof Element && t.classList.contains("cm-merge-a"));
    const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
    const folder =
      typeof cwd === "string"
        ? cwd
        : sessionId.startsWith("files-workspace:")
          ? sessionId.slice("files-workspace:".length)
          : "";
    const display = doc.shown.display;
    const words = savedWords(doc);
    const close = () => {
      files.workspace.close(sessionId, pair.left);
    };
    const savedText =
      doc.savedName === "commit"
        ? `The file ${display} as it was in the commit ${doc.savedLabel ?? ""}, in ${folder}. Read it with git show ${doc.savedLabel ?? "HEAD"}:./${display}.`
        : doc.savedName === "disk"
          ? `The file ${display} as it is on disk now, in ${folder} (the editor holds other, unsaved edits).`
          : `The last committed version of ${display}, in ${folder}. Read it with git show HEAD:./${display}.`;
    return codeMenu(
      app,
      {
        owner: sessionId,
        folder,
        path: display,
        unsaved: draft !== undefined && draft.text !== draft.base.text,
        editor: !left,
        saved: left
          ? { words: `${display} · ${words.line}`, tab: words.tab, text: savedText, close }
          : null,
        compare: { back: close },
      },
      viewSurface(left ? merge.a : merge.b),
    );
  };
  return {
    triggers: {
      onContextMenu: (event) => {
        const built = build(event.nativeEvent.composedPath());
        if (built === null) return;
        event.preventDefault();
        event.stopPropagation();
        menu.open({ x: event.clientX, y: event.clientY, target: built });
      },
      onKeyDown: (event) => {
        if (!isMenuKey(event)) return;
        const built = build(event.nativeEvent.composedPath());
        if (built === null) return;
        event.preventDefault();
        event.stopPropagation();
        const box = (event.target as Element).getBoundingClientRect();
        menu.open({ x: box.left + 24, y: box.top + 32, target: built });
      },
    },
    element:
      menu.at === null
        ? null
        : createPortal(
            <RowMenu
              testId="files.compare-menu"
              at={menu.at}
              label={menu.at.target.label}
              heading={menu.at.target.heading}
              items={menu.at.target.items}
              onClose={menu.close}
            />,
            document.body,
          ),
  };
}
