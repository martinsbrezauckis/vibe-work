/**
 * Modal focus handling: initial focus, Tab containment, inert background, Escape, and focus
 * restoration. Adapted from ShellX's modal focus hook.
 * Depends on: react. Called by: every modal surface (command palette, settings).
 * Invariants:
 * - While a modal is open, everything outside it is `inert` and `aria-hidden`, restored exactly
 *   (including elements that already had those) on close. The one exception is the dialog's own scrim
 *   (a sibling with class `scrim`, the dimmed backdrop): it is `aria-hidden` but stays clickable, so "a click
 *   outside closes" works. An inert scrim is not hit-tested at all (HTML: inert nodes act as
 *   `pointer-events: none`), which silently broke every scrim's `onClick`. The app's announcement regions
 *   (`data-announcer-root`) are left alone: they hold no controls and must stay audible.
 * - Tab and Shift+Tab wrap inside the dialog; focus returns to the element that had it before.
 * - Escape closes (capture phase), after an owned selector menu has handled its own dismissal.
 */
import { type RefObject, useEffect, useRef } from "react";

import { lockModalScroll } from "./modalScrollLock";

const FOCUSABLE = [
  "button:not([disabled])",
  "[href]",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) =>
      el.tabIndex >= 0 &&
      el.getClientRects().length > 0 &&
      el.getAttribute("aria-hidden") !== "true",
  );
}

/** True for the dimmed backdrop a dialog renders next to itself (it closes the dialog on a click). */
function isScrim(element: HTMLElement): boolean {
  return element.classList.contains("scrim");
}

/** A portalled selector belongs to the modal containing its trigger. */
function selectorOwner(element: Element | null, dialog: HTMLElement): HTMLElement | null {
  const portal = element?.closest<HTMLElement>("[data-selector-trigger]");
  const id = portal?.dataset.selectorTrigger;
  const owner = id ? document.getElementById(id) : null;
  return owner && dialog.contains(owner) ? owner : null;
}

/** Makes every sibling branch outside `dialog` inert (its scrim only `aria-hidden`); returns the undo function. */
function inertOutside(dialog: HTMLElement): () => void {
  const changed: { element: HTMLElement; inert: boolean; ariaHidden: string | null }[] = [];
  let kept: HTMLElement = dialog;
  let parent = kept.parentElement;
  while (parent && parent !== document.documentElement) {
    for (const child of Array.from(parent.children)) {
      if (child === kept || !(child instanceof HTMLElement)) continue;
      if (selectorOwner(child, dialog)) continue;
      // The app's announcement regions keep speaking while a dialog is open (a prompt can arrive meanwhile).
      if (child.hasAttribute("data-announcer-root")) continue;
      changed.push({
        element: child,
        inert: child.inert,
        ariaHidden: child.getAttribute("aria-hidden"),
      });
      // The scrim beside the dialog stays hit-testable: inert would swallow the click that closes the dialog.
      if (!(kept === dialog && isScrim(child))) child.inert = true;
      child.setAttribute("aria-hidden", "true");
    }
    kept = parent;
    parent = parent.parentElement;
  }
  return () => {
    for (const { element, inert, ariaHidden } of changed.reverse()) {
      element.inert = inert;
      if (ariaHidden === null) element.removeAttribute("aria-hidden");
      else element.setAttribute("aria-hidden", ariaHidden);
    }
  };
}

/** Applies the focus rules to `dialogRef` while `open`. `onClose` runs on Escape. */
export function useModalFocus(
  open: boolean,
  dialogRef: RefObject<HTMLElement | null>,
  onClose: () => void,
): void {
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const restore = inertOutside(dialog);
    const unlockScroll = lockModalScroll(dialog);
    // The marked initial field takes focus at once: waiting for a frame lost the first keystrokes
    // typed right after opening (seen under load: Ctrl K, "terminal" -> only "l" arrived). The
    // first-focusable fallback still waits a frame, because it needs layout to know what's visible.
    const preferred = dialog.querySelector<HTMLElement>('[data-dialog-initial-focus="true"]');
    preferred?.focus({ preventScroll: true });
    const frame = window.requestAnimationFrame(() => {
      if (dialog.contains(document.activeElement) || selectorOwner(document.activeElement, dialog))
        return;
      (preferred ?? focusables(dialog)[0] ?? dialog).focus({ preventScroll: true });
    });

    const onKey = (event: KeyboardEvent) => {
      const owner = selectorOwner(event.target instanceof Element ? event.target : null, dialog);
      // The selector handles Escape and closes on Tab. Move from its trigger after that closes the
      // portal, keeping the modal's containment and normal visual tab order.
      if (owner) {
        if (event.key === "Tab") {
          event.preventDefault();
          queueMicrotask(() => {
            moveFocus(owner, event.shiftKey);
          });
        }
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      if (
        event.target instanceof Element &&
        event.target.closest('[data-dialog-manages-tab="true"]')
      )
        return;
      event.preventDefault();
      moveFocus(document.activeElement, event.shiftKey);
    };

    const moveFocus = (active: Element | null, backwards: boolean) => {
      const items = focusables(dialog);
      const first = items[0];
      const last = items[items.length - 1];
      if (first === undefined || last === undefined) {
        dialog.focus({ preventScroll: true });
        return;
      }
      const index = items.findIndex((el) => el === active);
      let target: HTMLElement;
      if (backwards) target = index <= 0 ? last : (items[index - 1] ?? last);
      else target = index < 0 || index === items.length - 1 ? first : (items[index + 1] ?? first);
      target.focus();
    };

    document.addEventListener("keydown", onKey, true);
    return () => {
      window.cancelAnimationFrame(frame);
      document.removeEventListener("keydown", onKey, true);
      restore();
      unlockScroll();
      if (previous?.isConnected) previous.focus({ preventScroll: true });
    };
  }, [dialogRef, open]);
}
