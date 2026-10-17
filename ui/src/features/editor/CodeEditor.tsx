/**
 * CodeMirror file editor mounted in a shadow root for VW's strict style CSP.
 * Depends on CodeMirror and local language/theme setup; called by FileEditor.
 */
import { useEffect, useLayoutEffect, useRef } from "react";
import { Compartment, EditorState } from "@codemirror/state";
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
  openSearchPanel,
  search,
  searchKeymap,
  searchPanelOpen,
} from "@codemirror/search";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { languageFor } from "./languages";
import { editorHighlight, editorTheme } from "./theme";

export interface EditorHandle {
  focus(): void;
  find(): void;
  goToLine(line: number): void;
  getLine(): number;
  getScroll(): number;
  setText(text: string): void;
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
}

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
          lineNumbers(),
          foldGutter(),
          bracketMatching(),
          indentOnInput(),
          drawSelection(),
          highlightActiveLine(),
          history(),
          search(),
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
    };
    callbacks.current.onReady?.(api);
    if (props.line !== null) api.goToLine(props.line);
    else editor.focus();
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
