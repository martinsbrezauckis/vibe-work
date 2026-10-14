/**
 * Browser popover sizing, focus and overlay registration; page bounds never change for chrome menus.
 * Depends on: browserOverlays, React. Called by: Toolbar, CaptureMenu, SaveVault and future chrome popovers.
 * Invariant: closing/unmounting releases its overlay; Escape returns focus to the opener.
 */
import { useId, useLayoutEffect, useRef, type KeyboardEvent, type RefObject } from "react";

import { registerBrowserOverlay } from "./browserOverlays";

/** Register an open popover, focus its first control, and dismiss on outside clicks or Escape. */
export function useBrowserPopup(
  key: string,
  open: boolean,
  trigger: RefObject<HTMLElement | null>,
  close: () => void,
): RefObject<HTMLDivElement | null> {
  const popup = useRef<HTMLDivElement>(null);
  const id = useId();
  useLayoutEffect(() => {
    if (!open) return;
    const element = popup.current;
    if (element === null) return;
    let active = true;
    const measure = () => {
      if (!active) return;
      const top = element.getBoundingClientRect().top;
      const floor = document.querySelector(".statusbar")?.getBoundingClientRect().top;
      const bottom = floor !== undefined && floor > 0 ? floor : window.innerHeight;
      element.style.maxHeight = `${String(Math.max(0, bottom - top - 8))}px`;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    window.addEventListener("resize", measure);
    document.addEventListener("scroll", measure, true);
    element.querySelector<HTMLElement>("button:not(:disabled), input, summary")?.focus();
    const unregister = registerBrowserOverlay(`${id}-${key}`, {
      element,
      trigger: trigger.current,
      close,
    });
    return () => {
      active = false;
      observer.disconnect();
      window.removeEventListener("resize", measure);
      document.removeEventListener("scroll", measure, true);
      unregister();
    };
  }, [id, key, open, popup, trigger, close]);
  return popup;
}

/** Arrow/Home/End navigation for a menu; nested menus own their keys. */
export function browserMenuKeys(event: KeyboardEvent<HTMLDivElement>): void {
  if ((event.target as HTMLElement).closest('[role="menu"]') !== event.currentTarget) return;
  const items = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), summary"),
  ).filter((item) => item.getClientRects().length > 0);
  const at = items.indexOf(document.activeElement as HTMLElement);
  let next: number;
  if (event.key === "ArrowDown") next = (at + 1) % items.length;
  else if (event.key === "ArrowUp") next = (at - 1 + items.length) % items.length;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = items.length - 1;
  else return;
  event.preventDefault();
  items[next]?.focus();
}
