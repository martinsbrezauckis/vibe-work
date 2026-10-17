/** Read-only side-by-side mine/disk comparison; lazy loaded by FileEditor on Compare. */
import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { syntaxHighlighting } from "@codemirror/language";
import { EditorView, lineNumbers } from "@codemirror/view";
import { MergeView } from "@codemirror/merge";
import { languageFor } from "./languages";
import { editorHighlight, editorTheme } from "./theme";

export interface MergeCompareProps {
  mine: string;
  disk: string;
  filename: string;
  onClose: () => void;
}

export function MergeCompare({ mine, disk, filename, onClose }: MergeCompareProps) {
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!host.current) return;
    const root = host.current.shadowRoot ?? host.current.attachShadow({ mode: "open" });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(
      ".cm-mergeView{height:100%;overflow:auto;display:flex}.cm-mergeViewEditors{display:flex;flex:1;min-width:0}.cm-mergeViewEditor{flex:1;min-width:0}.cm-editor{height:100%}",
    );
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheet];
    let active = true;
    let merge: MergeView | null = null;
    void languageFor(filename).then((language) => {
      if (!active) return;
      const extensions = [
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        lineNumbers(),
        editorTheme,
        syntaxHighlighting(editorHighlight),
        language,
      ];
      merge = new MergeView({
        root,
        parent: root,
        a: {
          doc: mine,
          extensions: [...extensions, EditorView.contentAttributes.of({ "aria-label": "Mine" })],
        },
        b: {
          doc: disk,
          extensions: [...extensions, EditorView.contentAttributes.of({ "aria-label": "Disk" })],
        },
      });
    });
    return () => {
      active = false;
      merge?.destroy();
      root.adoptedStyleSheets = root.adoptedStyleSheets.filter((item) => item !== sheet);
    };
  }, [mine, disk, filename]);
  return (
    <section className="file-merge-compare" aria-label={`Compare ${filename}`}>
      <div className="file-merge-bar">
        <button data-testid="files.merge-compare-close" type="button" onClick={onClose}>
          Close comparison
        </button>
      </div>
      <div className="file-merge-labels">
        <strong>Mine</strong>
        <strong>Disk</strong>
      </div>
      <div ref={host} className="file-merge-view" data-merge-compare="" />
    </section>
  );
}
