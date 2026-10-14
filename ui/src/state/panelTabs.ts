/**
 * Registry of side panel tab bodies: features register what a tab shows (Project, Browser,
 * Changes, Terminal) without editing the shell. Until then each tab shows an honest empty
 * state (`features/shell/PlaceholderTabs.tsx`).
 * Depends on: react types, shellState (`TabKind`). Called by: the shell (registers placeholders),
 * feature modules (register real bodies), SidePanel (renders).
 * Invariants: one body per tab kind; registering again replaces (a feature takes over the
 * placeholder) and the returned function restores nothing, it only removes that registration.
 * A feature may hide a tab's button for now (`setHidden`); the header and the "+" menu skip it.
 */
import type { ReactNode } from "react";

import { createStore } from "./createStore";
import type { TabKind } from "./shellState";

/**
 * A tab body: a function that returns the element to show (usually `<MyTab />`). It is called in
 * render, so it must not use hooks itself; the component it returns can.
 */
export type TabBody = () => ReactNode;

/** Creates a registry. */
export function createPanelTabs() {
  const bodies = new Map<TabKind, TabBody>();
  /** Tab kinds the UI asked not to offer right now. */
  const hidden = createStore<readonly TabKind[]>([]);
  return {
    hidden,
    /** Offers (`false`) or stops offering (`true`) the button for `kind`; an open tab stays open. */
    setHidden(kind: TabKind, hide: boolean): void {
      hidden.set((now) =>
        hide === now.includes(kind) ? now : hide ? [...now, kind] : now.filter((k) => k !== kind),
      );
    },
    /** Sets the body for `kind`; returns the function that removes it. */
    register(kind: TabKind, body: TabBody): () => void {
      bodies.set(kind, body);
      return () => {
        if (bodies.get(kind) === body) bodies.delete(kind);
      };
    },
    /** The body for `kind`, if one is registered. */
    body(kind: TabKind): TabBody | undefined {
      return bodies.get(kind);
    },
  };
}

/** The registry the app uses. */
export type PanelTabs = ReturnType<typeof createPanelTabs>;
