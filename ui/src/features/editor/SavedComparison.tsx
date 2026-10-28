/** Editable comparison with the saved version on the left. CodeMirror aligns changed lines; the workspace owns the pair. */
import { useEffect, useLayoutEffect, useRef, type CSSProperties } from "react";
import { EditorState, Compartment } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { history, historyKeymap, defaultKeymap } from "@codemirror/commands";
import { syntaxHighlighting } from "@codemirror/language";
import { search, searchKeymap } from "@codemirror/search";
import { MergeView } from "@codemirror/merge";
import { Button, IconButton } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import type { Files, FileReader } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { EditorDivider } from "../files/EditorDivider";
import { saveEditor } from "../files/saveEditor";
import { savedWords } from "../files/openFiles";
import { useCompareMenu } from "./useCompareMenu";
import { languageFor } from "./languages";
import { editorHighlight, editorTheme } from "./theme";
import { editorActions } from "./actions";
import { registerCodeActions } from "../files/liveEditors";

/** Closing the saved tab restores the editable file without replacing its buffer. */
export function SavedComparison({
  files,
  reader,
  sessionId,
}: {
  files: Files;
  reader: FileReader;
  sessionId: string;
}) {
  const app = useApp();
  const view = useStore(files.workspace.store, (s) => s[sessionId]);
  const draft = useStore(reader.store, (s) => s.edits[sessionId]);
  const tab = useStore(reader.store, (s) => s.tabs[sessionId]);
  const pair = view?.split;
  const savedName = pair ? (view.documents[pair.left]?.savedName ?? "saved") : "saved";
  const words = savedWords(pair ? view.documents[pair.left] : undefined);
  const readLabel =
    savedName === "saved"
      ? "last saved version, read-only"
      : savedName === "commit"
        ? words.line
        : "Disk version, read-only";
  const saved = pair ? view.documents[pair.left]?.saved : undefined;
  const content = tab?.content;
  const container = useRef<HTMLDivElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const engine = useRef<MergeView | null>(null);
  const callbacks = useRef({ files, reader, sessionId, app });
  const writable = useRef(new Compartment());
  // Right click, the menu key or Shift F10 in either side: the compare's menu.
  const compareMenu = useCompareMenu(app, files, reader, sessionId, engine);
  useLayoutEffect(() => {
    callbacks.current = { files, reader, sessionId, app };
  });
  useEffect(() => {
    if (!host.current || saved === undefined || !content) return;
    reader.edit.begin(sessionId);
    const root = host.current.shadowRoot ?? host.current.attachShadow({ mode: "open" });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(
      ".cm-mergeView{height:100%;display:flex;overflow:hidden}.cm-mergeViewEditors{display:flex;flex:1;width:100%;height:100%;min-width:0;min-height:0}.cm-mergeView .cm-mergeViewEditor{flex:0 0 var(--saved-left,50%);height:100%;min-width:0;overflow:hidden}.cm-mergeView .cm-mergeViewEditor+.cm-mergeViewEditor{flex:1}.cm-mergeView .cm-editor{height:100% !important;overflow:hidden !important}.cm-mergeView .cm-editor .cm-scroller{height:100% !important;overflow-y:auto !important}.cm-merge-a{visibility:var(--saved-left-visible,visible)}.cm-merge-b{visibility:var(--saved-right-visible,visible)}@media(max-width:640px){.cm-mergeView .cm-mergeViewEditor{padding-top:44px;box-sizing:border-box}.cm-mergeViewEditors{flex-direction:column}.cm-mergeView .cm-mergeViewEditor{width:100%;min-height:0}}",
    );
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheet];
    const common = [
      EditorState.allowMultipleSelections.of(true),
      lineNumbers(),
      editorTheme,
      syntaxHighlighting(editorHighlight),
      search(),
      keymap.of([
        ...searchKeymap,
        {
          key: "Escape",
          run: () => {
            callbacks.current.app.shell.actions.unfoldChat();
            return true;
          },
        },
      ]),
    ];
    const leftTheme = EditorView.theme({
      "&.cm-merge-a .cm-changedLine": { backgroundColor: "var(--red-soft)" },
      "&.cm-merge-a .cm-changedText": {
        background: "var(--red-soft)",
        textDecoration: "underline",
        textDecorationColor: "var(--red)",
      },
      "&.cm-merge-a .cm-changedLineGutter": { background: "var(--red)" },
    });
    const rightTheme = EditorView.theme({
      "&.cm-merge-b .cm-changedLine": { backgroundColor: "var(--green-soft)" },
      "&.cm-merge-b .cm-changedText": {
        background: "var(--green-soft)",
        textDecoration: "underline",
        textDecorationColor: "var(--green)",
      },
      "&.cm-merge-b .cm-changedLineGutter": { background: "var(--green)" },
    });
    const merge = new MergeView({
      root,
      parent: root,
      highlightChanges: true,
      gutter: true,
      a: {
        doc: saved,
        extensions: [
          ...common,
          leftTheme,
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          EditorView.contentAttributes.of({ "aria-label": readLabel }),
          EditorView.domEventHandlers({
            focus: () => {
              callbacks.current.files.workspace.focus(sessionId, "left");
            },
          }),
        ],
      },
      b: {
        doc: reader.store.get().edits[sessionId]?.text ?? content.text ?? "",
        extensions: [
          ...common,
          rightTheme,
          history(),
          writable.current.of(EditorState.readOnly.of(false)),
          EditorView.contentAttributes.of({ "aria-label": `Text of ${content.name}` }),
          keymap.of([
            {
              key: "Mod-s",
              run: () => {
                void saveEditor(callbacks.current.app, sessionId, "ui", reader);
                return true;
              },
            },
            ...historyKeymap,
            ...defaultKeymap,
          ]),
          EditorView.domEventHandlers({
            focus: () => {
              callbacks.current.files.workspace.focus(sessionId, "right");
            },
          }),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) reader.edit.update(sessionId, { text: u.state.doc.toString() });
          }),
        ],
      },
    });
    engine.current = merge;
    const unregister = pair
      ? [
          registerCodeActions(sessionId, pair.left, editorActions(merge.a)),
          registerCodeActions(sessionId, pair.right, editorActions(merge.b)),
        ]
      : [];
    const syncLeft = () => {
      if (merge.b.scrollDOM.scrollTop !== merge.a.scrollDOM.scrollTop)
        merge.b.scrollDOM.scrollTop = merge.a.scrollDOM.scrollTop;
    };
    const syncRight = () => {
      if (merge.a.scrollDOM.scrollTop !== merge.b.scrollDOM.scrollTop)
        merge.a.scrollDOM.scrollTop = merge.b.scrollDOM.scrollTop;
    };
    merge.a.scrollDOM.addEventListener("scroll", syncLeft);
    merge.b.scrollDOM.addEventListener("scroll", syncRight);
    let alive = true;
    void languageFor(content.name).then((lang) => {
      if (alive) {
        merge.a.dispatch({ effects: StateEffect.appendConfig.of(lang) });
        merge.b.dispatch({ effects: StateEffect.appendConfig.of(lang) });
      }
    });
    return () => {
      for (const remove of unregister) remove();
      alive = false;
      merge.a.scrollDOM.removeEventListener("scroll", syncLeft);
      merge.b.scrollDOM.removeEventListener("scroll", syncRight);
      merge.destroy();
      engine.current = null;
      root.adoptedStyleSheets = root.adoptedStyleSheets.filter((s) => s !== sheet);
    };
    // The comparison is mounted per saved snapshot; draft updates flow through the view below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saved, savedName, content?.file, reader, sessionId, readLabel]);
  useEffect(() => {
    const editor = engine.current?.b;
    if (!editor) return;
    const text = draft?.text ?? content?.text ?? "";
    if (editor.state.doc.toString() !== text)
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } });
    editor.dispatch({
      effects: writable.current.reconfigure(EditorState.readOnly.of(draft?.saving ?? false)),
    });
  }, [draft?.text, draft?.saving, content?.text]);
  if (!pair || saved === undefined) return null;
  return (
    <section
      className="saved-comparison"
      aria-label={
        savedName === "saved"
          ? "Compare with saved version"
          : savedName === "commit"
            ? "Compare with this commit"
            : "Compare mine vs disk"
      }
      style={
        {
          "--saved-left": view.folded.includes("left")
            ? "48px"
            : view.folded.includes("right")
              ? "calc(100% - 48px)"
              : `${String(view.ratio * 100)}%`,
          "--saved-left-visible": view.folded.includes("left") ? "hidden" : "visible",
          "--saved-right-visible": view.folded.includes("right") ? "hidden" : "visible",
        } as CSSProperties
      }
    >
      <div className="saved-compare-headers">
        <div>
          <IconButton
            data-testid="files.compare-fold-saved"
            icon="panel-left"
            label={view.folded.includes("left") ? "Show saved editor" : "Fold saved editor"}
            onClick={() => {
              files.workspace.fold(sessionId, "left");
            }}
          />
          {!view.folded.includes("left") && (
            <>
              <span>{words.line}</span>
              <Button
                data-testid="files.close-saved-version"
                appearance="compact"
                onClick={() => files.workspace.close(sessionId, pair.left)}
              >
                {savedName === "disk" ? "Close comparison" : "Close saved version"}
              </Button>
            </>
          )}
        </div>
        <div>
          <IconButton
            data-testid="files.compare-fold-current"
            icon="panel-left"
            label={view.folded.includes("right") ? "Show editor" : "Fold editor"}
            onClick={() => {
              files.workspace.fold(sessionId, "right");
            }}
          />
          {!view.folded.includes("right") && (
            <>
              <span>{content?.name}</span>
              <Button
                data-testid="files.compare-save"
                appearance="compact"
                disabled={draft?.saving ?? false}
                onClick={() => void saveEditor(app, sessionId, "ui", reader)}
              >
                Save
              </Button>
            </>
          )}
        </div>
      </div>
      {draft?.conflict && (
        <p className="server-note" role="alert">
          Changed on disk. Your edits are still here. Close the comparison to review the file before
          saving.
        </p>
      )}
      {draft?.problem && (
        <p className="key-problem" role="alert">
          {draft.problem}
        </p>
      )}
      <div className="saved-compare-engine" ref={container}>
        <div
          className="saved-compare-host"
          ref={host}
          data-code-editor=""
          data-merge-compare={savedName === "disk" ? "" : undefined}
          {...compareMenu.triggers}
        />
        {compareMenu.element}
        <EditorDivider
          files={files}
          sessionId={sessionId}
          ratio={view.ratio}
          container={container}
        />
      </div>
    </section>
  );
}
import { StateEffect } from "@codemirror/state";
