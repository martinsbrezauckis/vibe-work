/**
 * The Project page's right-click menu: Point Vibe at this,
 * Comment… and, on selected text, Copy, in the app's one menu. The page is a native view that paints above this
 * window, so the page only reports where the right click was and whether it was on selected text (`pageMenu`); the
 * menu opens there while the page is parked under it, and each item reads what the click was on from the core.
 * Depends on: components/RowMenu, store.ts (menu events and reads), protocol types. Called by: usePagesTab (the
 * hook) and PagesTab (the menu).
 * Invariants: nothing of the page is read before an item is chosen; the menu belongs to the session shown and closes
 * when the page goes away; Point Vibe at this only adds a chip (never sends).
 */
import { type ReactElement, type RefObject, useCallback, useEffect, useState } from "react";

import { RowMenu, menuGroups } from "../../components/RowMenu";
import type { Rect } from "../../protocol/core";
import type { Pages } from "./store";

/** Where the menu is open (window coordinates) and on what. */
export interface PageMenuOpen {
  x: number;
  y: number;
  selected: boolean;
}

/**
 * The open state of `sessionId`'s page menu: a right click on its page opens it at that point of `area` (the page
 * area last reported to the core). Closed when no page is shown.
 */
export function usePageMenu(
  pages: Pages,
  sessionId: string,
  showingPage: boolean,
  area: RefObject<Rect | null>,
) {
  const [at, setAt] = useState<PageMenuOpen | null>(null);
  useEffect(
    () =>
      pages.onPageMenu((menu) => {
        const box = area.current;
        if (menu.sessionId !== sessionId || !showingPage || box === null) return;
        setAt({
          x: box.x + Math.min(menu.x, box.width),
          y: box.y + Math.min(menu.y, box.height),
          selected: menu.selected,
        });
      }),
    [pages, sessionId, showingPage, area],
  );
  const close = useCallback(() => {
    setAt(null);
  }, []);
  // No page shown (the hub, another page loading): no menu.
  return { at: showingPage ? at : null, close };
}

/** The menu. `path` is the page shown (the header names it). */
export function PageMenu({
  at,
  path,
  onClose,
  point,
  comment,
  copy,
}: {
  at: PageMenuOpen;
  path: string;
  onClose: () => void;
  point: () => void;
  comment: () => void;
  copy: () => void;
}): ReactElement {
  return (
    <RowMenu
      testId="pages.page-menu"
      at={at}
      label={`Actions for ${path}`}
      heading={at.selected ? `${path} · selected text` : `${path} · this section`}
      onClose={onClose}
      items={menuGroups(
        [
          { id: "point", label: "Point Vibe at this", run: point },
          { id: "comment", label: "Comment…", run: comment },
        ],
        [at.selected && { id: "copy", label: "Copy", run: copy, keys: "Mod+C" }],
      )}
    />
  );
}
