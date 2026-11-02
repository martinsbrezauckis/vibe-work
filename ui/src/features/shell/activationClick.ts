/** Protects ask cards from window-activation clicks. Uses capture listeners; installed by shell wiring and removed on unmount. */

// A click that activates the window arrives within moments of the window's focus; a press later than this after
// the window came back (a native dialog closed, a switch with the keyboard) is an ordinary click.
const ACTIVATION_MS = 400;

/**
 * Whether a press on a card is the one that activated the window: after the window lost focus, before or right after
 * the focus came back. The Debug API's ui/click is judged the same way, as a person's click would be.
 */
export function isActivationPress(p: {
  activating: boolean;
  focusedAt: number | null;
  now: number;
  onApproval: boolean;
}): boolean {
  if (!p.activating || !p.onApproval) return false;
  return p.focusedAt === null || p.now - p.focusedAt <= ACTIVATION_MS;
}

/**
 * Whether a pointerdown is an accessibility press made by the browser engine: Chromium (Edge, WebView2) turns a
 * Windows UI Automation Invoke (Voice Access, Narrator) into a trusted mouse pointerdown with a button down and no
 * pressure, while a person's mouse press always has pressure 0.5 while a button is down. Such a press is never an
 * activation click (it does not bring the window forward), so it is never held back. Only a trusted event counts:
 * a page script or the Debug API cannot make one.
 */
export function isAccessibilityPress(p: {
  isTrusted: boolean;
  pointerType: string;
  isPrimary: boolean;
  buttons: number;
  pressure: number;
}): boolean {
  return (
    p.isTrusted && p.pointerType === "mouse" && p.isPrimary && p.buttons !== 0 && p.pressure === 0
  );
}

export function guardActivationClick(): () => void {
  let activating = false;
  let focusedAt: number | null = null;
  let blocked = false;
  const blur = () => {
    activating = true;
    focusedAt = null;
  };
  const focus = (event: Event) => {
    focusedAt = event.timeStamp;
  };
  const cancel = (event: Event) => {
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  const down = (event: Event) => {
    if (event instanceof PointerEvent && isAccessibilityPress(event)) {
      // Answered as pressed; the window keeps waiting for the person's own activation click.
      blocked = false;
      return;
    }
    blocked = isActivationPress({
      activating,
      focusedAt,
      now: event.timeStamp,
      onApproval: event.target instanceof Element && event.target.closest(".approval") !== null,
    });
    activating = false;
    if (blocked) cancel(event);
  };
  const finish = (event: Event) => {
    if (!blocked) return;
    cancel(event);
    // The held-back press ends with its click (or its context menu). A later press that comes without a pointer,
    // an accessibility press (Voice Control, VoiceOver, Orca: mouse events and a click only), is never held back.
    if (event.type === "click" || event.type === "contextmenu") blocked = false;
  };
  const cancelled = () => {
    blocked = false;
  };
  const key = () => {
    activating = false;
    blocked = false;
  };
  window.addEventListener("blur", blur);
  window.addEventListener("focus", focus);
  window.addEventListener("pointerdown", down, true);
  window.addEventListener("mousedown", finish, true);
  window.addEventListener("mouseup", finish, true);
  window.addEventListener("click", finish, true);
  window.addEventListener("contextmenu", finish, true);
  window.addEventListener("keydown", key, true);
  window.addEventListener("pointercancel", cancelled, true);
  return () => {
    window.removeEventListener("blur", blur);
    window.removeEventListener("focus", focus);
    window.removeEventListener("pointerdown", down, true);
    window.removeEventListener("mousedown", finish, true);
    window.removeEventListener("mouseup", finish, true);
    window.removeEventListener("click", finish, true);
    window.removeEventListener("contextmenu", finish, true);
    window.removeEventListener("keydown", key, true);
    window.removeEventListener("pointercancel", cancelled, true);
  };
}
