/**
 * Vitest setup: registers the jest-dom matchers (toBeInTheDocument, ...), unmounts rendered trees
 * between tests, and gives jsdom a layout answer it lacks: `getClientRects()` returns one empty
 * rect for every element, so the focus-trap's "is this element visible" check (ported from ShellX)
 * works. Elements hidden with the `hidden` attribute still report none. Loaded by
 * `test.setupFiles` in both UI Vitest configurations.
 *
 * `ResizeObserver`: jsdom has none, and the transcript's virtualised list (virtua) needs
 * one to learn its viewport size. The stub reports every observed element as 100 000 px tall, so the
 * whole list fits the "viewport" and every row is mounted (jsdom has no layout: virtua never measures
 * items there, because their `offsetParent` is null, and keeps its size estimate). `scrollTo` is a
 * no-op for the same reason.
 *
 * The app's announcer (`ui/src/state/announcer.ts`) is emptied after each test, and text queries skip its regions.
 */
import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach, beforeEach } from "vitest";
import { resetAnnouncer } from "../../../ui/src/state/announcer";

// `findBy*` / `waitFor` wait for a condition, so their limit is only a failure bound. The 1 s default
// is shorter than a cold dynamic import under CPU load; a generous bound removes that race without
// slowing passing tests (they resolve the moment the element exists).
// The app's announcement regions repeat visible words for screen readers only; text queries look at what is shown.
configure({
  asyncUtilTimeout: 30_000,
  defaultIgnore:
    "script, style, [data-announcer-root], [data-announcer-root] *",
});

class TestResizeObserver {
  private readonly callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  observe(target: Element): void {
    queueMicrotask(() => {
      this.callback(
        [
          {
            target,
            contentRect: { width: 1000, height: 100_000 },
          } as unknown as ResizeObserverEntry,
        ],
        this,
      );
    });
  }

  unobserve(): void {}

  disconnect(): void {}
}
window.ResizeObserver = TestResizeObserver;
// jsdom has no element scrolling; virtua scrolls its viewport to follow the newest row.
Element.prototype.scrollTo = function scrollTo(): void {};
Element.prototype.scrollIntoView = function scrollIntoView(): void {};

// jsdom has no modal-dialog API. Model its open/close state; native tests verify focus and pixels.
HTMLDialogElement.prototype.showModal = function showModal(): void {
  this.setAttribute("open", "");
};
HTMLDialogElement.prototype.close = function close(): void {
  this.removeAttribute("open");
  this.dispatchEvent(new Event("close"));
};
// jsdom has the popover hiding rule but no top-layer API. Browser tests verify native showing.
HTMLElement.prototype.showPopover = function showPopover(): void {
  this.style.display = "block";
};
HTMLElement.prototype.hidePopover = function hidePopover(): void {
  this.style.display = "none";
};

beforeEach(() => {
  localStorage.clear();
  HTMLElement.prototype.getClientRects = function getClientRects(
    this: HTMLElement,
  ) {
    return this.hidden
      ? ([] as unknown as DOMRectList)
      : ([{}] as unknown as DOMRectList);
  };
});

afterEach(() => {
  cleanup();
  resetAnnouncer();
});
