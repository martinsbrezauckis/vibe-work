/**
 * Opens the code menu (`codeMenu.ts`) of the File tab's text: in the editor (right click, the menu key or Shift
 * F10 at the caret) and in the files viewer (right click on the text). Holds the menu's position and renders it.
 * Depends on: RowMenu, codeMenu, files state, selection reader, AppContext. Called by: FileTab's SessionFile.
 */
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactElement,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { RowMenu, isMenuKey, useRowMenu } from "../../components/RowMenu";
import { type FileReader, tabOf } from "../../state/files";
import type { EditorHandle } from "../editor/CodeEditor";
import type { App } from "../shell/AppContext";
import { type CodeSurface, codeMenu } from "./codeMenu";
import { readFileSelection } from "./selection";

type Built = ReturnType<typeof codeMenu>;

/** The editor handle as a code surface. */
function handleSurface(handle: EditorHandle, readOnly: boolean): CodeSurface {
  return {
    selection: () => handle.selection(),
    line: () => handle.getLine(),
    lineText: (line) => handle.lineText(line),
    replace: (text) => handle.replaceSelection(text),
    find: () => {
      handle.find();
    },
    goToLine: () => {
      handle.goToLinePrompt();
    },
    readOnly,
  };
}

/** The menu for the File tab of `sessionId`; spread `triggers` on the text's container. */
export function useCodeMenu(
  app: App,
  files: FileReader,
  sessionId: string,
  root: RefObject<HTMLDivElement | null>,
  engine: RefObject<EditorHandle | null>,
): {
  triggers: {
    onContextMenu: (event: ReactMouseEvent) => void;
    onKeyDown: (event: ReactKeyboardEvent) => void;
  };
  element: ReactElement | null;
} {
  const menu = useRowMenu<Built>();
  const build = (pointerLine: number | null): Built | null => {
    const state = files.store.get();
    const tab = tabOf(state, sessionId);
    const draft = state.edits[sessionId];
    const content = draft?.base ?? tab.content;
    if (tab.shown === null || content === null || content.text === null) return null;
    const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
    const folder =
      typeof cwd === "string"
        ? cwd
        : sessionId.startsWith("files-workspace:")
          ? sessionId.slice("files-workspace:".length)
          : "";
    const workspace = app.files?.workspace;
    const pair = workspace?.of(sessionId).split;
    const place = {
      owner: sessionId,
      folder,
      path: content.display,
      unsaved: draft !== undefined && draft.text !== draft.base.text,
      editor: draft !== undefined,
      compare:
        workspace === undefined
          ? null
          : pair?.compare === true
            ? {
                back: () => {
                  workspace.close(sessionId, pair.left);
                },
              }
            : {
                open: () => {
                  void workspace.compareSaved(sessionId);
                },
              },
    };
    if (draft !== undefined) {
      const handle = engine.current;
      if (handle === null) return null;
      return codeMenu(app, place, handleSurface(handle, draft.saving));
    }
    const text = content.text.split("\n");
    const viewer: CodeSurface = {
      selection: () => (root.current === null ? null : readFileSelection(root.current)),
      line: () => pointerLine ?? 1,
      lineText: (line) => text[line - 1] ?? "",
      replace: () => false,
      find: () => {
        app.find.actions.openFile(sessionId);
      },
      readOnly: true,
    };
    return codeMenu(app, place, viewer);
  };
  const lineAt = (target: EventTarget | null): number | null => {
    const line = (target instanceof Element ? target : null)?.closest<HTMLElement>("[data-line]");
    return line?.dataset["line"] === undefined ? null : Number(line.dataset["line"]);
  };
  return {
    triggers: {
      onContextMenu: (event) => {
        const built = build(lineAt(event.target));
        if (built === null) return;
        event.preventDefault();
        event.stopPropagation();
        menu.open({ x: event.clientX, y: event.clientY, target: built });
      },
      onKeyDown: (event) => {
        if (!isMenuKey(event)) return;
        const built = build(lineAt(event.target));
        if (built === null) return;
        event.preventDefault();
        event.stopPropagation();
        // At the caret when the editor shows one, else at the text's top left.
        const caret = document.querySelector(".cm-focused .cm-cursor")?.getBoundingClientRect();
        const box = (event.target as Element).getBoundingClientRect();
        menu.open({
          x: caret?.left ?? box.left + 16,
          y: (caret?.bottom ?? box.top + 24) + 2,
          target: built,
        });
      },
    },
    element:
      menu.at === null
        ? null
        : createPortal(
            <RowMenu
              testId="files.code-menu"
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
