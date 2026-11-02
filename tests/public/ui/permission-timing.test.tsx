/** Permission arrival tests exercise typing, keyboard grace and deliberate pointer answers through AskCard. */
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AskCard } from "../../../ui/src/features/ask/AskCard";
import { startPromptActivity } from "../../../ui/src/features/ask/permissionArrival";

let stop: () => void;
beforeEach(() => {
  vi.useFakeTimers();
  stop = startPromptActivity();
});
afterEach(() => {
  stop();
  vi.useRealTimers();
});

const buttons = [{ key: 1, choice: "once", label: "Allow once" }] as const;
const card = (id: string, choose: () => void) => (
  <AskCard
    id={id}
    question="Run this command?"
    who="Vibe"
    buttons={buttons}
    disabled={false}
    state=""
    onChoose={choose}
  />
);
const advance = (ms: number) => {
  act(() => {
    vi.advanceTimersByTime(ms);
  });
};

it("waits for a full second after the last keystroke before showing a new permission", () => {
  const choose = vi.fn();
  const view = render(card("previous", choose));
  fireEvent.keyDown(document, { key: "x" });
  view.rerender(card("arriving", choose));
  expect(screen.queryByRole("group")).toBeNull();
  advance(900);
  fireEvent.keyDown(document, { key: "y" });
  advance(999);
  expect(screen.queryByRole("group")).toBeNull();
  advance(1);
  expect(screen.getByRole("group")).toBeVisible();
  expect(choose).not.toHaveBeenCalled();
});

it("ignores answer keys and keyboard button clicks for 500 ms after arrival", () => {
  const choose = vi.fn();
  render(card("keyboard-grace", choose));
  const frame = screen.getByRole("group");
  const button = screen.getByRole("button", { name: "Allow once" });
  fireEvent.keyDown(frame, { key: "1" });
  fireEvent.click(button, { detail: 0 });
  expect(choose).not.toHaveBeenCalled();
  advance(499);
  fireEvent.keyDown(frame, { key: "1" });
  expect(choose).not.toHaveBeenCalled();
  advance(1);
  fireEvent.keyDown(frame, { key: "1" });
  expect(choose).toHaveBeenCalledOnce();
});

it("accepts a deliberate pointer click while the keyboard grace is active", () => {
  const choose = vi.fn();
  render(card("pointer-grace", choose));
  fireEvent.click(screen.getByRole("button", { name: "Allow once" }), {
    detail: 1,
  });
  expect(choose).toHaveBeenCalledOnce();
});

it("suppresses a held Enter key's native button activation after the grace period", () => {
  const choose = vi.fn();
  render(card("held-enter", choose));
  advance(501);
  const allowedDefault = fireEvent.keyDown(screen.getByRole("button"), {
    key: "Enter",
    repeat: true,
  });
  expect(allowedDefault).toBe(false);
  expect(choose).not.toHaveBeenCalled();
});

// The Debug API's ui/click (ui/src/features/shell/inputRoutes.ts) is a pointer press and release followed by the
// element's own click(), whose `detail` is 0 like a keyboard activation. A click that completes a pointer press on
// the button is a pointer click and answers during the keyboard grace.
it("accepts a pointer press and release ending in a detail-0 click while the keyboard grace is active", () => {
  const choose = vi.fn();
  render(card("pointer-sequence-grace", choose));
  const button = screen.getByRole("button", { name: "Allow once" });
  fireEvent.pointerDown(button, {
    button: 0,
    isPrimary: true,
    pointerType: "mouse",
  });
  fireEvent.mouseDown(button, { button: 0 });
  fireEvent.pointerUp(button, {
    button: 0,
    isPrimary: true,
    pointerType: "mouse",
  });
  fireEvent.mouseUp(button, { button: 0 });
  fireEvent.click(button, { detail: 0 });
  expect(choose).toHaveBeenCalledOnce();
});

it("still ignores a keyboard click after a pointer press that left the button", () => {
  const choose = vi.fn();
  render(card("pointer-left-grace", choose));
  const button = screen.getByRole("button", { name: "Allow once" });
  fireEvent.pointerDown(button, {
    button: 0,
    isPrimary: true,
    pointerType: "mouse",
  });
  fireEvent.pointerLeave(button, { isPrimary: true, pointerType: "mouse" });
  fireEvent.click(button, { detail: 0 });
  expect(choose).not.toHaveBeenCalled();
});
