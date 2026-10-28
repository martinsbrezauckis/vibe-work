/**
 * Empty bodies for side panel tabs until each feature registers its view. A feature registers
 * its own body over this one.
 * Depends on: state/panelTabs, state/shellState. Called by: main.tsx.
 */
import { TEXT as WORDS } from "../../vocabulary";
import type { PanelTabs } from "../../state/panelTabs";
import { TAB_KINDS, type TabKind } from "../../state/shellState";

const TEXT: Record<TabKind, string> = {
  project: WORDS.handbookPlaceholder,
  plan: "The plan's steps will show here.",
  browser: "The browser will show here.",
  git: "Choose a folder to see its files.",
  changes: "File changes will show here.",
  terminal: "The terminal will show here.",
  file: "Click a file name in the chat to preview it here.",
};

/** Registers the placeholder bodies. Returns the function that removes them. */
export function registerPlaceholderTabs(tabs: PanelTabs): () => void {
  const removers = TAB_KINDS.map((kind) =>
    tabs.register(kind, () => <div className="empty">{TEXT[kind]}</div>),
  );
  return () => {
    for (const remove of removers) remove();
  };
}
