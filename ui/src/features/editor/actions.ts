/** Runtime editor operations shared by the CodeMirror handle and Debug API routes; reads the actual editor state. */
import { EditorState } from "@codemirror/state";
import { type EditorView } from "@codemirror/view";
import { foldable, foldedRanges, foldEffect, unfoldEffect } from "@codemirror/language";
import {
  SearchQuery,
  setSearchQuery,
  getSearchQuery,
  openSearchPanel,
  closeSearchPanel,
  searchPanelOpen,
  findNext,
  findPrevious,
  selectMatches,
  replaceNext,
  replaceAll,
} from "@codemirror/search";

export interface EditorSearch {
  query: string;
  replace?: string;
  caseSensitive?: boolean;
  regexp?: boolean;
  wholeWord?: boolean;
}

import type { SearchCommand } from "./searchCommands";

export { SEARCH_COMMANDS, type SearchCommand } from "./searchCommands";

/** Uses CodeMirror transactions and commands, including the same query options as its built-in search panel. */
export function editorActions(editor: EditorView) {
  return {
    goToLine(line: number): void {
      const at = editor.state.doc.line(Math.max(1, Math.min(line, editor.state.doc.lines))).from;
      editor.dispatch({ selection: { anchor: at }, scrollIntoView: true });
      editor.focus();
    },
    search(options: EditorSearch): void {
      openSearchPanel(editor);
      editor.dispatch({
        effects: setSearchQuery.of(new SearchQuery({ ...options, search: options.query })),
      });
    },
    searchCommand(command: SearchCommand): boolean {
      if (editor.state.readOnly && (command === "replaceNext" || command === "replaceAll"))
        return false;
      return {
        next: findNext,
        previous: findPrevious,
        selectAll: selectMatches,
        replaceNext,
        replaceAll,
        close: closeSearchPanel,
      }[command](editor);
    },
    foldCode(line: number, folded: boolean): boolean {
      if (!Number.isInteger(line) || line < 1 || line > editor.state.doc.lines) return false;
      const at = editor.state.doc.line(line);
      const ranges = foldedRanges(editor.state);
      const existing: { from: number; to: number }[] = [];
      ranges.between(at.from, at.to, (from, to) => {
        existing.push({ from, to });
      });
      if (!folded) {
        editor.dispatch({ effects: existing.map((range) => unfoldEffect.of(range)) });
        return existing.length > 0;
      }
      if (existing.length > 0) return true;
      const range = foldable(editor.state, at.from, at.to);
      if (range === null) return false;
      editor.dispatch({ effects: foldEffect.of(range) });
      return true;
    },
    codeState() {
      const query = getSearchQuery(editor.state);
      let matches = 0;
      if (query.valid) {
        const cursor = query.getCursor(editor.state);
        while (!cursor.next().done) matches++;
      }
      const folds: { from: number; to: number; line: number }[] = [];
      foldedRanges(editor.state).between(0, editor.state.doc.length, (from, to) => {
        folds.push({ from, to, line: editor.state.doc.lineAt(from).number });
      });
      return {
        readOnly: editor.state.facet(EditorState.readOnly),
        selection: { from: editor.state.selection.main.from, to: editor.state.selection.main.to },
        selections: editor.state.selection.ranges.map(({ from, to }) => ({ from, to })),
        cursorLine: editor.state.doc.lineAt(editor.state.selection.main.head).number,
        folds,
        search: {
          query: query.search,
          replace: query.replace,
          caseSensitive: query.caseSensitive,
          regexp: query.regexp,
          wholeWord: query.wholeWord,
          valid: query.valid,
          matches,
          open: searchPanelOpen(editor.state),
        },
      };
    },
  };
}
