/**
 * The editor's search commands by name, for the `ui/editors` route and the CodeMirror actions. Kept free of
 * CodeMirror imports: the route is registered at startup, and importing `actions.ts` there would load the editor
 * before the first paint.
 */
export const SEARCH_COMMANDS = [
  "next",
  "previous",
  "selectAll",
  "replaceNext",
  "replaceAll",
  "close",
] as const;
export type SearchCommand = (typeof SEARCH_COMMANDS)[number];
