/** Positions the context panel above its meter; owns focus, dismissal and native-view reservation.
 * Depends on React and nativeLayer; called only while ContextComposition is mounted.
 */
import { type RefObject, useLayoutEffect } from "react";
import { reserve } from "../../components/nativeLayer";

export function useContextPopover(
  panel: RefObject<HTMLDivElement | null>,
  trigger: RefObject<HTMLButtonElement | null>,
  close: () => void,
) {
  useLayoutEffect(() => {
    const element = panel.current;
    const opener = trigger.current;
    if (!element || !opener) return;
    let restore = true;
    element.showPopover({ source: opener });
    const measure = () => {
      const anchor = opener.getBoundingClientRect();
      const inset = 8;
      // App text size scales the document; DOM rectangles are already scaled, CSS positions are not.
      const zoom = Number(document.documentElement.style.zoom) || 1;
      element.style.setProperty(
        "max-height",
        `${String(Math.max(0, anchor.top - inset * 2) / zoom)}px`,
      );
      element.style.setProperty("max-width", `${String((window.innerWidth - inset * 2) / zoom)}px`);
      const bounds = element.getBoundingClientRect();
      element.style.setProperty(
        "top",
        `${String(Math.max(inset, anchor.top - bounds.height - inset) / zoom)}px`,
      );
      element.style.setProperty(
        "left",
        `${String(
          Math.max(
            inset,
            Math.min(anchor.right - bounds.width, window.innerWidth - bounds.width - inset),
          ) / zoom,
        )}px`,
      );
      reserve("context-composition", element.getBoundingClientRect());
    };
    measure();
    element.focus({ preventScroll: true });
    let frame = 0;
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    observer?.observe(element);
    observer?.observe(opener);
    const keydown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      close();
    };
    const outside = (event: Event) => {
      if (
        !(event.target instanceof Node) ||
        element.contains(event.target) ||
        opener.contains(event.target)
      )
        return;
      restore = false;
      close();
    };
    document.addEventListener("keydown", keydown, true);
    document.addEventListener("pointerdown", outside);
    document.addEventListener("focusin", outside);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    return () => {
      observer?.disconnect();
      cancelAnimationFrame(frame);
      document.removeEventListener("keydown", keydown, true);
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("focusin", outside);
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      element.hidePopover();
      reserve("context-composition", null);
      if (restore && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, [panel, trigger, close]);
}
