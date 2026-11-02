/** The window-activation guard holds back only the press that activates the window, never a later one. */
import { afterEach, beforeEach, expect, it } from "vitest";
import {
  guardActivationClick,
  isAccessibilityPress,
  isActivationPress,
} from "../../../ui/src/features/shell/activationClick";

let stop: () => void;
let card: HTMLButtonElement;
let clicks: number;
beforeEach(() => {
  stop = guardActivationClick();
  const frame = document.createElement("div");
  frame.className = "approval";
  card = document.createElement("button");
  frame.append(card);
  document.body.append(frame);
  clicks = 0;
  card.addEventListener("click", () => {
    clicks += 1;
  });
});
afterEach(() => {
  stop();
  document.body.replaceChildren();
});

const press = (target: Element) => {
  target.dispatchEvent(
    new PointerEvent("pointerdown", {
      bubbles: true,
      isPrimary: true,
      button: 0,
    }),
  );
  target.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  target.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  (target as HTMLElement).click();
};

it("the press that activates the window does not answer the card; the next press does", () => {
  window.dispatchEvent(new Event("blur"));
  press(card);
  expect(clicks).toBe(0);
  press(card);
  expect(clicks).toBe(1);
});

// What an accessibility press (AT-SPI doAction, macOS AXPress: Voice Control, VoiceOver, Orca) dispatches in
// WebKit: mouse events and the click, with no pointer events and a click count of 0.
const accessibilityPress = (target: Element) => {
  target.dispatchEvent(
    new MouseEvent("mousedown", { bubbles: true, cancelable: true }),
  );
  target.dispatchEvent(
    new MouseEvent("mouseup", { bubbles: true, cancelable: true }),
  );
  target.dispatchEvent(
    new MouseEvent("click", { bubbles: true, cancelable: true, detail: 0 }),
  );
};

it("an accessibility press after a held-back activation press answers the card", () => {
  window.dispatchEvent(new Event("blur"));
  press(card);
  expect(clicks).toBe(0);
  accessibilityPress(card);
  expect(clicks).toBe(1);
});

it("an accessibility press while the window has no focus answers the card", () => {
  window.dispatchEvent(new Event("blur"));
  accessibilityPress(card);
  expect(clicks).toBe(1);
});

it("a press is an activation press only before or right after the window gets its focus back", () => {
  const onCard = { onApproval: true };
  expect(
    isActivationPress({
      ...onCard,
      activating: true,
      focusedAt: null,
      now: 1000,
    }),
  ).toBe(true);
  expect(
    isActivationPress({
      ...onCard,
      activating: true,
      focusedAt: 990,
      now: 1000,
    }),
  ).toBe(true);
  // The window came back long before (a native dialog closed, a switch with the keyboard): an ordinary click.
  expect(
    isActivationPress({
      ...onCard,
      activating: true,
      focusedAt: 1000,
      now: 5000,
    }),
  ).toBe(false);
  expect(
    isActivationPress({
      ...onCard,
      activating: false,
      focusedAt: null,
      now: 1000,
    }),
  ).toBe(false);
  expect(
    isActivationPress({
      ...onCard,
      onApproval: false,
      activating: true,
      focusedAt: null,
      now: 1000,
    }),
  ).toBe(false);
});

// What Chromium (Edge, WebView2) dispatches for a Windows UI Automation Invoke (Voice Access, Narrator), measured on
// Windows 11: a trusted mouse pointerdown with a button down but no pressure, at the control's centre, then the
// click. A person's mouse press has a button down and pressure 0.5.
const uiaPointerdown = {
  isTrusted: true,
  pointerType: "mouse",
  isPrimary: true,
  buttons: 1,
  pressure: 0,
};

it("tells a Windows accessibility press from a person's mouse press", () => {
  expect(isAccessibilityPress(uiaPointerdown)).toBe(true);
  // A person's mouse press (also the one that activates the window).
  expect(isAccessibilityPress({ ...uiaPointerdown, pressure: 0.5 })).toBe(
    false,
  );
  // Page-made or Debug API events are never trusted, whatever they claim.
  expect(isAccessibilityPress({ ...uiaPointerdown, isTrusted: false })).toBe(
    false,
  );
  // No button down (a hover), a pen or a touch, a second pointer.
  expect(isAccessibilityPress({ ...uiaPointerdown, buttons: 0 })).toBe(false);
  expect(isAccessibilityPress({ ...uiaPointerdown, pointerType: "pen" })).toBe(
    false,
  );
  expect(
    isAccessibilityPress({ ...uiaPointerdown, pointerType: "touch" }),
  ).toBe(false);
  expect(isAccessibilityPress({ ...uiaPointerdown, isPrimary: false })).toBe(
    false,
  );
});

it("an untrusted press shaped like an accessibility press is still held back as an activation press", () => {
  // What a page script or the Debug API can make: the same fields, but never a trusted event.
  window.dispatchEvent(new Event("blur"));
  card.dispatchEvent(
    new PointerEvent("pointerdown", {
      bubbles: true,
      cancelable: true,
      isPrimary: true,
      pointerType: "mouse",
      buttons: 1,
      pressure: 0,
    }),
  );
  card.click();
  expect(clicks).toBe(0);
});
