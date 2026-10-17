/**
 * Real CodeMirror helpers for jsdom integration tests. Depends on EditorView and testing-library;
 * called by file-draft tests. Transactions exercise the editor's actual change callback.
 */
import { waitFor } from "@testing-library/react";
import { EditorView } from "@codemirror/view";

/** Supply browser APIs that jsdom does not implement; browser tests verify layout and styles. */
export function setupCodeEditor(): void {
  Object.defineProperty(document, "execCommand", {
    configurable: true,
    value: () => false,
  });
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
  CSSStyleSheet.prototype.replaceSync = () => {};
  Object.defineProperty(ShadowRoot.prototype, "adoptedStyleSheets", {
    configurable: true,
    get(this: ShadowRoot & { sheets?: CSSStyleSheet[] }) {
      return this.sheets ?? [];
    },
    set(
      this: ShadowRoot & { sheets?: CSSStyleSheet[] },
      value: CSSStyleSheet[],
    ) {
      this.sheets = value;
    },
  });
}

/** Find the live editor through its shadow root, including after a session remount. */
export async function codeEditor(): Promise<EditorView> {
  return waitFor(() => {
    const node = document
      .querySelector("[data-code-editor]")
      ?.shadowRoot?.querySelector<HTMLElement>(".cm-editor");
    const view = node == null ? null : EditorView.findFromDOM(node);
    if (!view) throw new Error("No live code editor");
    return view;
  });
}
