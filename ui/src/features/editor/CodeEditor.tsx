/**
 * CodeMirror file editor mounted in a shadow root for VW's strict style CSP.
 * Depends on CodeMirror and local language/theme setup; called by FileEditor.
 * `marks` lines (changed on disk while the file was open) carry a line class that moves with later typing.
 */
import { useEffect, useLayoutEffect, useRef } from "react";
import { Compartment, EditorState, StateEffect, StateField, type Text } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import {
  closeSearchPanel,
  gotoLine,
  openSearchPanel,
  search,
  searchKeymap,
  searchPanelOpen,
} from "@codemirror/search";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  drawSelection,
  highlightActiveLine,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { languageFor } from "./languages";
import { editorHighlight, editorTheme } from "./theme";
import type { CodeSelection } from "../files/model/context";
import { editorActions } from "./actions";

export interface EditorHandle extends ReturnType<typeof editorActions> {
  /** Reads the main selection without changing the buffer, caret or focus. */
  selection(): CodeSelection | null;
  focus(): void;
  find(): void;
  goToLine(line: number): void;
  getLine(): number;
  getScroll(): number;
  setText(text: string): void;
  /** The text of line `line` (1-based), for pointing at the caret's line (right-click menu). */
  lineText(line: number): string;
  /** Replaces the selection (Cut, Paste in the right-click menu); false in a read-only editor. */
  replaceSelection(text: string): boolean;
  /** Opens the editor's Go to line field. */
  goToLinePrompt(): void;
}

export interface CodeEditorProps {
  value: string;
  onChange: (text: string) => void;
  label: string;
  filename: string;
  line: number | null;
  readOnly?: boolean;
  wrap?: boolean;
  onSave?: () => void;
  onCancel?: () => void;
  onReady?: (api: EditorHandle | null) => void;
  /** Lines to mark as changed on disk (1-based; empty: none). */
  marks?: readonly number[];
}

const setMarks = StateEffect.define<readonly number[]>();

function markSet(doc: Text, lines: readonly number[]): DecorationSet {
  const mark = Decoration.line({ class: "cm-vibe-line" });
  return Decoration.set(
    lines.filter((n) => n >= 1 && n <= doc.lines).map((n) => mark.range(doc.line(n).from)),
  );
}

/** The marked lines, mapped through every later change so they stay on the same text. */
const marksField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    let next = value.map(transaction.changes);
    for (const effect of transaction.effects)
      if (effect.is(setMarks)) next = markSet(transaction.state.doc, effect.value);
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

function linePosition(view: EditorView): { line: number; column: number } {
  const pos = view.state.selection.main.head;
  const line = view.state.doc.lineAt(pos);
  return { line: line.number, column: pos - line.from };
}

function atLine(view: EditorView, number: number, column = 0): number {
  const line = view.state.doc.line(Math.max(1, Math.min(number, view.state.doc.lines)));
  return Math.min(line.to, line.from + column);
}

/** Bypass browser rich-text mutation when typing over a selected code range. */
export function replaceSelectedText(event: InputEvent, current: EditorView): boolean {
  const selection = current.state.selection.main;
  if (
    selection.empty ||
    current.state.readOnly ||
    event.isComposing ||
    !event.cancelable ||
    !event.isTrusted
  )
    return false;
  if (event.inputType !== "insertText" || event.data === null) return false;
  event.preventDefault();
  current.dispatch({
    ...current.state.replaceSelection(event.data),
    userEvent: "input.type",
  });
  return true;
}

/** A single live view; prop changes reconfigure compartments or dispatch document changes. */
export function CodeEditor(props: CodeEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const callbacks = useRef(props);
  useLayoutEffect(() => {
    callbacks.current = props;
  });
  const compartments = useRef({
    language: new Compartment(),
    readOnly: new Compartment(),
    theme: new Compartment(),
    wrap: new Compartment(),
    label: new Compartment(),
  });
  const suppressChange = useRef(false);

  useEffect(() => {
    if (!host.current) return;
    const root = host.current.shadowRoot ?? host.current.attachShadow({ mode: "open" });
    const parts = compartments.current;
    const editor = new EditorView({
      root,
      parent: root,
      state: EditorState.create({
        doc: props.value,
        extensions: [
          EditorState.allowMultipleSelections.of(true),
          lineNumbers(),
          foldGutter(),
          bracketMatching(),
          indentOnInput(),
          drawSelection(),
          highlightActiveLine(),
          history(),
          search(),
          marksField,
          syntaxHighlighting(editorHighlight),
          keymap.of([
            {
              key: "Mod-s",
              run: () => {
                callbacks.current.onSave?.();
                return true;
              },
            },
            {
              key: "Escape",
              run: (v) => {
                if (searchPanelOpen(v.state)) return closeSearchPanel(v);
                if (callbacks.current.onCancel) {
                  callbacks.current.onCancel();
                  return true;
                }
                return false;
              },
            },
            ...searchKeymap,
            ...historyKeymap,
            ...foldKeymap,
            ...defaultKeymap,
          ]),
          parts.language.of([]),
          parts.readOnly.of([
            EditorState.readOnly.of(!!props.readOnly),
            EditorView.editable.of(!props.readOnly),
          ]),
          parts.theme.of(editorTheme),
          parts.wrap.of(props.wrap ? EditorView.lineWrapping : []),
          parts.label.of(EditorView.contentAttributes.of({ "aria-label": props.label })),
          EditorView.domEventHandlers({ beforeinput: replaceSelectedText }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged && !suppressChange.current)
              callbacks.current.onChange(update.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = editor;
    const api: EditorHandle = {
      ...editorActions(editor),
      selection: () => {
        const { from, to, empty } = editor.state.selection.main;
        if (empty) return null;
        return {
          start: editor.state.doc.lineAt(from).number,
          end: editor.state.doc.lineAt(Math.max(from, to - 1)).number,
          text: editor.state.sliceDoc(from, to),
        };
      },
      focus: () => {
        editor.focus();
      },
      find: () => {
        openSearchPanel(editor);
        // The first call installs the panel; the second focuses its input.
        openSearchPanel(editor);
      },
      goToLine: (line) => {
        const anchor = atLine(editor, line);
        editor.dispatch({
          selection: { anchor },
          effects: EditorView.scrollIntoView(anchor, { y: "center" }),
        });
        editor.focus();
      },
      getLine: () => linePosition(editor).line,
      getScroll: () => editor.scrollDOM.scrollTop,
      setText: (text) => {
        if (editor.state.readOnly || editor.state.doc.toString() === text) return;
        editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } });
      },
      lineText: (line) =>
        editor.state.doc.line(Math.max(1, Math.min(line, editor.state.doc.lines))).text,
      replaceSelection: (text) => {
        if (editor.state.readOnly) return false;
        editor.dispatch(editor.state.replaceSelection(text));
        editor.focus();
        return true;
      },
      goToLinePrompt: () => {
        gotoLine(editor);
      },
    };
    callbacks.current.onReady?.(api);
    if (props.line !== null) {
      const anchor = atLine(editor, props.line);
      editor.dispatch({
        selection: { anchor },
        effects: EditorView.scrollIntoView(anchor, { y: "center" }),
      });
    }
    // A lazy editor must not take focus back after the user moved into the message field.
    const active = document.activeElement;
    if (!(
      active instanceof HTMLElement &&
      (active.matches("input, textarea") || active.isContentEditable)
    ))
      editor.focus();
    return () => {
      callbacks.current.onReady?.(null);
      editor.destroy();
      view.current = null;
    };
    // The view belongs to this mount. Effects below own prop updates.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const oldText = editor.state.doc.toString();
    const value = editor.state.toText(props.value).toString();
    if (oldText === value) return;
    const { line, column } = linePosition(editor);
    // Preserve unchanged lines in the document mapping, including the viewport anchor.
    let from = 0;
    while (from < oldText.length && from < value.length && oldText[from] === value[from]) from++;
    let oldTo = oldText.length;
    let newTo = value.length;
    while (oldTo > from && newTo > from && oldText[oldTo - 1] === value[newTo - 1]) {
      oldTo--;
      newTo--;
    }
    const changes = { from, to: oldTo, insert: value.slice(from, newTo) };
    suppressChange.current = true;
    try {
      const transaction = editor.state.update({ changes });
      const target = transaction.newDoc.line(Math.max(1, Math.min(line, transaction.newDoc.lines)));
      const scrollTo = editor.scrollSnapshot().map(transaction.changes);
      editor.dispatch({
        changes,
        selection: { anchor: Math.min(target.to, target.from + column) },
        effects: scrollTo ?? [],
      });
    } finally {
      suppressChange.current = false;
    }
  }, [props.value]);

  // After the text sync above, so the marks are placed on the new text.
  const marks = (props.marks ?? []).join(",");
  useEffect(() => {
    const editor = view.current;
    if (editor)
      editor.dispatch({
        effects: setMarks.of(marks === "" ? [] : marks.split(",").map(Number)),
      });
  }, [marks]);

  useEffect(() => {
    const editor = view.current;
    if (editor)
      editor.dispatch({
        effects: compartments.current.readOnly.reconfigure([
          EditorState.readOnly.of(!!props.readOnly),
          EditorView.editable.of(!props.readOnly),
        ]),
      });
  }, [props.readOnly]);

  useEffect(() => {
    const editor = view.current;
    if (editor)
      editor.dispatch({
        effects: compartments.current.wrap.reconfigure(props.wrap ? EditorView.lineWrapping : []),
      });
  }, [props.wrap]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const applyTheme = () => {
      editor.dispatch({
        effects: compartments.current.theme.reconfigure([
          editorTheme,
          EditorView.theme({
            "&": {
              colorScheme: document.documentElement.dataset["theme"] === "light" ? "light" : "dark",
            },
          }),
        ]),
      });
    };
    applyTheme();
    const observer = new MutationObserver(applyTheme);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => {
      observer.disconnect();
    };
  }, []);

  const previousLine = useRef(props.line);
  useEffect(() => {
    if (props.line !== null && props.line !== previousLine.current && view.current) {
      const anchor = atLine(view.current, props.line);
      view.current.dispatch({
        selection: { anchor },
        effects: EditorView.scrollIntoView(anchor, { y: "center" }),
      });
    }
    previousLine.current = props.line;
  }, [props.line]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    let active = true;
    void languageFor(props.filename).then((language) => {
      if (active && view.current === editor)
        editor.dispatch({ effects: compartments.current.language.reconfigure(language) });
    });
    return () => {
      active = false;
    };
  }, [props.filename]);

  useEffect(() => {
    const editor = view.current;
    if (editor)
      editor.dispatch({
        effects: compartments.current.label.reconfigure(
          EditorView.contentAttributes.of({ "aria-label": props.label }),
        ),
      });
  }, [props.label]);

  return (
    <div
      ref={host}
      data-testid="files.file-editor-set-text"
      className="file-edit-area"
      data-code-editor=""
      data-filename={props.filename}
    />
  );
}
