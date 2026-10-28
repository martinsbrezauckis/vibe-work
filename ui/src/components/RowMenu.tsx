/**
 * The app's one action menu (role `menu`), opened at a point: the row's ⋯ button, a right click, the
 * menu key or Shift F10 all open the same menu (`useRowMenu` wires the three). The header line names
 * the target; items keep one order everywhere (open, Point Vibe at this, versions, copy and show,
 * destructive last: `menuGroups`), and an item's shortcut is shown at its right. Arrow keys move,
 * Enter or click runs, Escape or an outside click closes and the focus returns to where it was.
 * Depends on: react, Icon, state/shortcuts (key words), state/openMenu (the Debug API's view) and
 * shared-controls.css. Called by: sessions, projects, files, editors, Changes, terminal and chat.
 * Invariants: positioned with `position: fixed` from the opener's rectangle, flipped up/left when
 * it would leave the window, so the scrolling list never clips it; items are plain buttons, a
 * disabled one is skipped by the arrow keys.
 */
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { detectMac, formatKeys } from "../state/shortcuts";
import { registerOpenMenu } from "../state/openMenu";
import { Icon } from "./Icon";

/** One entry. */
export interface MenuItem {
  id: string;
  label: ReactNode;
  name?: string;
  run: () => void;
  /** Destructive: shown in the danger colour. */
  danger?: boolean;
  /** Greyed out and skipped. */
  disabled?: boolean;
  /** A rule is drawn above this item. */
  separated?: boolean;
  /** A sibling action on a chooser row, also available with Shift+Enter. */
  secondary?: { label: string; run: () => void; disabled?: boolean };
  /** The item's shortcut as the shortcut list writes it ("Mod+L", "Enter"); shown at the right. */
  keys?: string;
}

/** Leaves out the items that cannot apply (`false`, `null`) and draws a rule above each later group. */
export function menuGroups(
  ...groups: readonly (MenuItem | false | null | undefined)[][]
): MenuItem[] {
  const out: MenuItem[] = [];
  for (const group of groups) {
    const items = group.filter((item): item is MenuItem => item != null && item !== false);
    items.forEach((item, index) => {
      out.push(index === 0 && out.length > 0 ? { ...item, separated: true } : item);
    });
  }
  return out;
}

/** The keyboard's way to a context menu: the menu key or Shift F10. */
export function isMenuKey(event: { key: string; shiftKey: boolean }): boolean {
  return event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
}

/** The shortcut in the words `aria-keyshortcuts` expects ("Control+L"). */
function ariaKeys(keys: string): string {
  return keys.replace(/\bMod\b/g, detectMac() ? "Meta" : "Control");
}

/** Where a menu opens and for which target. */
export interface MenuAt<T> {
  x: number;
  y: number;
  target: T;
}

/**
 * The open state of one menu with its three openers: `onContextMenu` (right click at the pointer),
 * `onKeyDown` (the menu key or Shift F10 at the focused element) and `openBelow` (the ⋯ button).
 */
export function useRowMenu<T>() {
  const [at, setAt] = useState<MenuAt<T> | null>(null);
  const openBelow = useCallback((element: Element, target: T) => {
    const box = element.getBoundingClientRect();
    setAt({ x: box.left, y: box.bottom + 2, target });
  }, []);
  const triggers = useCallback(
    (target: T) => ({
      onContextMenu: (event: ReactMouseEvent) => {
        event.preventDefault();
        event.stopPropagation();
        setAt({ x: event.clientX, y: event.clientY, target });
      },
      onKeyDown: (event: ReactKeyboardEvent) => {
        if (!isMenuKey(event)) return;
        event.preventDefault();
        event.stopPropagation();
        const box = event.currentTarget.getBoundingClientRect();
        setAt({ x: box.left + 16, y: box.top + Math.min(box.height, 28), target });
      },
    }),
    [],
  );
  const close = useCallback(() => {
    setAt(null);
  }, []);
  return { at, open: setAt, openBelow, triggers, close };
}

/** One menu's state and openers, as `useRowMenu` returns them (passed down to the rows that open it). */
export type RowMenuState<T> = ReturnType<typeof useRowMenu<T>>;

/** A row's ⋯ button: opens the row's menu below itself. */
export function RowMenuButton({
  testId,
  itemId,
  label,
  expanded,
  onOpen,
}: {
  testId: string;
  itemId: string;
  label: string;
  expanded: boolean;
  onOpen: (button: HTMLButtonElement) => void;
}) {
  return (
    <button
      data-testid={testId}
      data-item-id={itemId}
      type="button"
      className="row-more"
      aria-label={label}
      title={label}
      aria-haspopup="menu"
      aria-expanded={expanded}
      tabIndex={-1}
      onClick={(event) => {
        event.stopPropagation();
        onOpen(event.currentTarget);
      }}
    >
      <Icon name="ellipsis" size={14} />
    </button>
  );
}

/** The menu. `at` is where to open it (viewport coordinates). */
export function RowMenu({
  at,
  label,
  items,
  onClose,
  heading,
  footer,
  testId = "shared.row-menu",
}: {
  at: { x: number; y: number };
  label: string;
  items: readonly MenuItem[];
  onClose: () => void;
  heading?: string | undefined;
  footer?: string | undefined;
  testId?: string;
}) {
  const root = useRef<HTMLDivElement>(null);
  const opener = useRef<Element | null>(document.activeElement);
  const [pos, setPos] = useState(at);

  useLayoutEffect(() => {
    const box = root.current?.getBoundingClientRect();
    if (box === undefined) return;
    setPos({
      x: Math.max(4, Math.min(at.x, window.innerWidth - box.width - 4)),
      y: Math.max(4, at.y + box.height > window.innerHeight ? at.y - box.height : at.y),
    });
    root.current?.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
  }, [at]);

  // The latest onClose without re-running the effects below on every render of the parent.
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    const away = (event: MouseEvent) => {
      if (root.current !== null && !root.current.contains(event.target as Node)) close.current();
    };
    // Escape closes the menu before any dialog the list sits in (the narrow-window drawer traps
    // Escape in the capture phase of `document`); `window` capture runs first.
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      close.current();
    };
    document.addEventListener("mousedown", away, true);
    window.addEventListener("keydown", escape, true);
    return () => {
      document.removeEventListener("mousedown", away, true);
      window.removeEventListener("keydown", escape, true);
    };
  }, []);
  // The Debug API sees the open menu and chooses items the way a click does.
  useLayoutEffect(() => {
    const text = (id: string) =>
      root.current?.querySelector(
        `[data-testid="shared.row-menu-item"][data-item-id="${CSS.escape(id)}"] .menu-label`,
      )?.textContent ?? id;
    return registerOpenMenu({
      testId,
      label,
      heading: heading ?? null,
      items: items.map((item) => ({
        id: item.id,
        label: item.name ?? text(item.id),
        keys: item.keys === undefined ? null : formatKeys(item.keys),
        disabled: item.disabled === true,
        danger: item.danger === true,
      })),
      choose: (id) => {
        const item = items.find((candidate) => candidate.id === id);
        if (item === undefined || item.disabled === true) return false;
        close.current();
        item.run();
        return true;
      },
      close: () => {
        close.current();
      },
    });
  }, [items, testId, label, heading]);
  // Focus returns to the opener only when the menu goes away.
  useEffect(() => {
    const previous = opener.current;
    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, []);

  const move = (step: 1 | -1) => {
    const buttons = [
      ...(root.current?.querySelectorAll<HTMLElement>("button:not(:disabled)") ?? []),
    ];
    if (buttons.length === 0) return;
    const current = buttons.indexOf(document.activeElement as HTMLElement);
    buttons[(current + step + buttons.length) % buttons.length]?.focus();
  };

  return (
    <div
      data-testid={testId}
      ref={root}
      className="menu"
      role="menu"
      aria-label={label}
      style={{ left: pos.x, top: pos.y }}
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") move(1);
        else if (e.key === "ArrowUp") move(-1);
        else if (e.key === "Tab") onClose();
        else return;
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {heading !== undefined && <div className="menu-heading">{heading}</div>}
      {items.map((item) => (
        <div className="menu-row" key={item.id}>
          <button
            data-testid="shared.row-menu-item"
            data-item-id={item.id}
            type="button"
            role="menuitem"
            aria-label={item.name ?? (typeof item.label === "string" ? item.label : undefined)}
            aria-keyshortcuts={item.keys === undefined ? undefined : ariaKeys(item.keys)}
            className={`${item.danger === true ? "danger" : ""}${item.separated === true ? " sep" : ""}`}
            disabled={item.disabled === true}
            onKeyDown={(event) => {
              if (event.key === "Enter" && event.shiftKey && item.secondary !== undefined) {
                event.preventDefault();
                event.stopPropagation();
                if (!item.secondary.disabled) {
                  onClose();
                  item.secondary.run();
                }
              }
            }}
            onClick={() => {
              onClose();
              item.run();
            }}
          >
            <span className="menu-label">{item.label}</span>
            {item.keys !== undefined && (
              <kbd className="menu-keys" aria-hidden="true">
                {formatKeys(item.keys)}
              </kbd>
            )}
          </button>
          {item.secondary !== undefined && (
            <button
              data-testid="shared.row-menu-edit"
              data-item-id={item.id}
              type="button"
              aria-label={item.secondary.label}
              title={item.secondary.label}
              disabled={item.secondary.disabled}
              onClick={() => {
                onClose();
                item.secondary?.run();
              }}
            >
              Edit
            </button>
          )}
        </div>
      ))}
      {footer !== undefined && <p className="menu-note">{footer}</p>}
    </div>
  );
}
