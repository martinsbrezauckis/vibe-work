/**
 * The message box's height: it grows with its wrapped text, line by line, up to `maxRows` lines, then scrolls.
 * Counting only typed line breaks (the earlier rule) left a long wrapped sentence in a one-line box that scrolled,
 * so the start of the message went out of sight while typing.
 * Depends on: React. Called by: Composer.
 * Invariants: measures the real content height (`scrollHeight`) after every value change and when the box's width
 * changes (wrapping changes with it); never taller than `maxRows` lines plus the box's own padding; the CSS
 * `min-height` still sets the smallest box.
 */
import { type RefObject, useLayoutEffect } from "react";

/** Line height used when the computed style has none (15px text at 1.5, the composer's font). */
const FALLBACK_LINE = 22.5;

/** Sets `el`'s height to its content, capped at `maxRows` lines; scrolls only past the cap. */
export function fitHeight(el: HTMLTextAreaElement, maxRows: number): void {
  const style = getComputedStyle(el);
  const line = Number.parseFloat(style.lineHeight) || FALLBACK_LINE;
  const padding =
    (Number.parseFloat(style.paddingTop) || 0) + (Number.parseFloat(style.paddingBottom) || 0);
  const max = Math.round(line * maxRows + padding);
  // Collapse first, so a shorter text can shrink the box (scrollHeight never reports less than the current height).
  el.style.height = "auto";
  const content = el.scrollHeight;
  el.style.height = `${String(Math.min(content, max))}px`;
  el.style.overflowY = content > max ? "auto" : "hidden";
}

/** Keeps the box in `box` fitted to `value`, also when its width changes. */
export function useAutoHeight(
  box: RefObject<HTMLTextAreaElement | null>,
  value: string,
  maxRows: number,
): void {
  useLayoutEffect(() => {
    const el = box.current;
    if (el !== null) fitHeight(el, maxRows);
  }, [box, value, maxRows]);
  useLayoutEffect(() => {
    const el = box.current;
    if (el === null || typeof ResizeObserver === "undefined") return;
    // Only a width change re-wraps the text; reacting to our own height change would loop.
    let width = el.clientWidth;
    const observer = new ResizeObserver(() => {
      if (el.clientWidth === width) return;
      width = el.clientWidth;
      fitHeight(el, maxRows);
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
    };
  }, [box, maxRows]);
}
