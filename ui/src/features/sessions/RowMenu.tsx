/**
 * A small action menu (role `menu`) opened at a point: the row's "more" button, a right click or
 * the context-menu key. Arrow keys move, Enter or click runs, Escape or an outside click closes and
 * the focus returns to where it was.
 * Depends on: react. Called by: SessionRow, ProjectGroupView, projects/useProjectActions.
 * Invariants: positioned with `position: fixed` from the opener's rectangle, flipped up/left when
 * it would leave the window, so the scrolling list never clips it; items are plain buttons, a
 * disabled one is skipped by the arrow keys.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** One entry. */
export interface MenuItem {
  id: string;
  label: string;
  run: () => void;
  /** Destructive: shown in the danger colour. */
  danger?: boolean;
  /** Greyed out and skipped. */
  disabled?: boolean;
  /** A rule is drawn above this item. */
  separated?: boolean;
}

/** The menu. `at` is where to open it (viewport coordinates). */
export function RowMenu({
  at,
  label,
  items,
  onClose,
}: {
  at: { x: number; y: number };
  label: string;
  items: readonly MenuItem[];
  onClose: () => void;
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
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          className={`${item.danger === true ? "danger" : ""}${item.separated === true ? " sep" : ""}`}
          disabled={item.disabled === true}
          onClick={() => {
            onClose();
            item.run();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
