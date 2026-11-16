/**
 * Whether an ask card's answer keys (1/2/3, Enter, Space) may answer now: only while the person's focus is
 * deliberately on a card. Keys that reach a card for any other reason answer nothing.
 *
 * Ends the keys (every card's): a press on the chat's built-in browser page while the agent controls it (the core's
 * `blockedPagePress`: the input block swallows that press natively, so the window's own focus never moves; the focus
 * is taken off the focused element too, so typed text reaches neither a card nor the composer), a press anywhere
 * in the window outside a card, and the window losing focus (another app, a native view of the app).
 * Gives them back: a focus that moves to another element inside a card while the window is in front (Tab, a click,
 * a card's own arrival focus), a press inside a card, and "Go to the waiting prompt" (`armCardKeys`). The focus the
 * engine restores to the same element when the window comes back is not a deliberate one.
 *
 * Depends on: the DOM. Called by: AskCard (asks and reports focus), the shell wiring (`startCardKeys`), the window
 * focus channel (`activationClick`).
 */
import type { CoreController } from "../../state/coreStore";

let armed = true;
/** The element focused when the keys ended: focus returning to it is not a fresh one. */
let endedOn: Element | null = null;
/** False from the window's blur until its focus. */
let windowActive = true;
/** True while the window's own focus event is dispatched: the element focus that follows it is a restored one. */
let reactivating = false;

/**
 * The digit an answer key press stands for, or null. The character the engine reports (`key`) wins; when it is not a
 * digit (a layout whose digit row types a symbol, a press without a character) the physical digit or keypad key
 * (`code`) stands in, unless Shift is held, which asks for the shifted symbol and not the number.
 */
export function answerDigit(event: {
  key: string;
  code: string;
  shiftKey: boolean;
}): string | null {
  if (/^[0-9]$/.test(event.key)) return event.key;
  if (event.shiftKey) return null;
  const physical = /^(?:Digit|Numpad)([0-9])$/.exec(event.code);
  return physical === null ? null : (physical[1] ?? null);
}

/** Whether a key may answer a card now. */
export function cardKeysArmed(): boolean {
  return armed;
}

/** Ends every card's answer keys until a deliberate focus. */
export function endCardKeys(): void {
  armed = false;
  endedOn = document.activeElement;
}

/** A deliberate request for a card's keyboard focus (the person asked for the waiting card). */
export function armCardKeys(): void {
  armed = true;
  endedOn = null;
}

/** Tells the rule that `target`, inside a card, received the focus. */
export function cardFocused(target: EventTarget): void {
  if (armed) return;
  if (!windowActive || reactivating) {
    // Focus moved while the window was not in front (an arrival focus behind another app): the person never saw it.
    if (!reactivating && target instanceof Element) endedOn = target;
    return;
  }
  if (target !== endedOn) armCardKeys();
}

/** The person pressed the chat's blocked built-in page: keys end and the focused element lets go of the focus. */
function blockedPagePress(): void {
  endCardKeys();
  const active = document.activeElement;
  if (active instanceof HTMLElement && active !== document.body) active.blur();
}

function inCard(event: Event): boolean {
  return event.target instanceof Element && event.target.closest(".approval") !== null;
}

/** Installs the window listeners and the core event; returns their removal. */
export function startCardKeys(core: Pick<CoreController, "listen"> | null): () => void {
  // Not capturing: an element's focus and blur do not bubble, so only the window's own reach these two.
  const blur = () => {
    windowActive = false;
    endCardKeys();
  };
  const focus = () => {
    windowActive = true;
    reactivating = true;
    setTimeout(() => {
      reactivating = false;
    }, 0);
  };
  // Capture: a press outside a card ends the keys even when a control stops the event.
  const downOutside = (event: Event) => {
    if (!inCard(event)) endCardKeys();
  };
  // Bubble: the activation click the window holds back (`activationClick`) never reaches here.
  const downInside = (event: Event) => {
    if (inCard(event)) armCardKeys();
  };
  window.addEventListener("blur", blur);
  window.addEventListener("focus", focus);
  window.addEventListener("pointerdown", downOutside, true);
  window.addEventListener("pointerdown", downInside);
  const stopEvents =
    core === null
      ? undefined
      : core.listen((event) => {
          if (event.type === "blockedPagePress") blockedPagePress();
        });
  return () => {
    stopEvents?.();
    window.removeEventListener("blur", blur);
    window.removeEventListener("focus", focus);
    window.removeEventListener("pointerdown", downOutside, true);
    window.removeEventListener("pointerdown", downInside);
    armed = true;
    endedOn = null;
    windowActive = true;
    reactivating = false;
  };
}
