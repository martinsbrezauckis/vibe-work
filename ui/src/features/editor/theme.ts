/** VW token-based CodeMirror styles; style-mod adopts these in the editor's shadow root. */
import { HighlightStyle } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";

export const editorHighlight = HighlightStyle.define([
  { tag: tags.keyword, class: "syn-token-keyword" },
  { tag: [tags.string, tags.regexp], class: "syn-token-string" },
  { tag: [tags.comment, tags.meta], class: "syn-token-comment" },
  { tag: [tags.number, tags.bool, tags.null], class: "syn-token-constant" },
  {
    tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
    class: "syn-token-function",
  },
  { tag: tags.variableName, class: "syn-token-parameter" },
  { tag: tags.punctuation, class: "syn-token-punctuation" },
  { tag: tags.link, class: "syn-token-link" },
]);

export const editorTheme = EditorView.theme({
  "&": { height: "100%", minHeight: "0", backgroundColor: "var(--bg)", color: "var(--text)" },
  ".cm-scroller": {
    overflow: "auto",
    fontFamily: "var(--mono)",
    fontSize: "12.5px",
    lineHeight: "1.55",
  },
  ".cm-content": { caretColor: "var(--text)", minHeight: "100%" },
  // drawSelection hides the native caret and draws these; their base colour is black unless the theme is marked
  // dark, so on the dark scheme the cursor all but vanished. The token follows both schemes.
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--text)" },
  ".cm-gutters": {
    backgroundColor: "var(--panel)",
    color: "var(--text-3)",
    borderColor: "var(--line)",
  },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "var(--active)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "var(--term-selection)",
  },
  ".cm-searchMatch": { backgroundColor: "var(--mark)" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--mark-on)" },
  ".cm-panels": {
    backgroundColor: "var(--panel)",
    color: "var(--text)",
    borderColor: "var(--line)",
  },
  ".cm-panels input": {
    backgroundColor: "var(--raised)",
    color: "var(--text)",
    borderColor: "var(--line-2)",
  },
  ".syn-token-keyword": { color: "var(--syn-keyword)" },
  ".syn-token-string": { color: "var(--syn-string)" },
  ".syn-token-comment": { color: "var(--syn-comment)" },
  ".syn-token-constant": { color: "var(--syn-constant)" },
  ".syn-token-function": { color: "var(--syn-function)" },
  ".syn-token-parameter": { color: "var(--syn-parameter)" },
  ".syn-token-punctuation": { color: "var(--syn-punctuation)" },
  ".syn-token-link": { color: "var(--syn-link)" },
});
