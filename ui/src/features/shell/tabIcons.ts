/**
 * The icon of each side-panel tab kind, shared by the session header buttons (Content) and the panel's own tab strip
 * (SidePanel) so a tab always shows the same picture in both places (one family, one meaning per icon).
 * Depends on: components/Icon (names), state/shellState (tab kinds). Called by: Content, SidePanel.
 */
import type { IconName } from "../../components/Icon";
import type { TabKind } from "../../state/shellState";

/** Tab kind → icon: the handbook (open book), the plan checklist (a checked list), the browser (globe), changes (file with +/-), terminal, a file. */
export const TAB_ICON: Readonly<Record<TabKind, IconName>> = {
  project: "book-open",
  plan: "list-todo",
  browser: "globe",
  changes: "file-diff",
  terminal: "square-terminal",
  file: "file",
};
