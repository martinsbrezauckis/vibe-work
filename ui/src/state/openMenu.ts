/**
 * The action menu that is open now, as the Debug API sees it: its container id, the header line that names the
 * target, and each item (id, words, shortcut, disabled, danger). Choosing runs the item exactly as a click does.
 * Depends on: nothing. Called by: components/RowMenu (registers while open) and the `ui/menu` route.
 * Invariant: at most one menu is open; a newer menu closes the previous one and replaces the record, and closing
 * clears it only when it is still the one that registered.
 */

/** One item as the route reports it. */
interface OpenMenuItem {
  id: string;
  label: string;
  keys: string | null;
  disabled: boolean;
  danger: boolean;
}

/** The open menu. */
export interface OpenMenu {
  testId: string;
  label: string;
  heading: string | null;
  items: OpenMenuItem[];
  choose: (id: string) => boolean;
  close: () => void;
}

let current: OpenMenu | null = null;

/** Records `menu` as the open one; the returned function forgets it again (only if still current). */
export function registerOpenMenu(menu: OpenMenu): () => void {
  // One menu at a time: a newer menu closes the one still open (a keyboard or Debug API opener sends no
  // outside click that would have closed it).
  const previous = current;
  current = menu;
  if (previous !== null && previous !== menu) previous.close();
  return () => {
    if (current === menu) current = null;
  };
}

/** The open menu, or null. */
export function openMenu(): OpenMenu | null {
  return current;
}
