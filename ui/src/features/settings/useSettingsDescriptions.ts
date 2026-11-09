/**
 * Connects visible Settings helper lines to their row's controls without changing control names or layout.
 * Called by SettingsPage; observes asynchronously loaded rows and preserves controls' own descriptions.
 */
import { type RefObject, useId, useLayoutEffect, useRef } from "react";

const ROW = ".check, .set-hit";
const CONTROL = "button, input, select, textarea, a[href]";

/** Keeps helper ids stable for the lifetime of their elements, including text and value updates. */
export function useSettingsDescriptions(root: RefObject<HTMLDivElement | null>): void {
  const prefix = useId();
  const serial = useRef(0);
  useLayoutEffect(() => {
    const page = root.current;
    if (page === null) return;
    const linked = new Map<HTMLElement, Set<string>>();
    const identify = (element: HTMLElement): string => {
      if (element.id === "") element.id = `${prefix}-help-${String(++serial.current)}`;
      return element.id;
    };
    const connect = () => {
      const next = new Map<HTMLElement, Set<string>>();
      for (const helper of page.querySelectorAll<HTMLElement>(".d")) {
        const id = identify(helper);
        const row = helper.closest<HTMLElement>(ROW);
        if (row === null) continue;
        const controls = row.matches(CONTROL)
          ? [row]
          : [...row.querySelectorAll<HTMLElement>(CONTROL)].filter(
              (control) => control.closest(ROW) === row && control.closest(".d") === null,
            );
        // Read-only rows have no control to describe. Give their visible details a native reading stop.
        if (controls.length === 0 && !helper.hasAttribute("role"))
          helper.setAttribute("role", "note");
        for (const control of controls) {
          const ids = next.get(control) ?? new Set<string>();
          ids.add(id);
          next.set(control, ids);
        }
        // A search hit contains both label and detail inside its button. Only the label names the action.
        if (row.matches(".set-hit") && !row.hasAttribute("aria-labelledby")) {
          const label = row.querySelector<HTMLElement>("b");
          if (label !== null) row.setAttribute("aria-labelledby", identify(label));
        }
      }
      for (const control of new Set([...linked.keys(), ...next.keys()])) {
        if (!page.contains(control)) {
          linked.delete(control);
          continue;
        }
        const previous = linked.get(control) ?? new Set<string>();
        const own = (control.getAttribute("aria-describedby") ?? "")
          .split(/\s+/)
          .filter((id) => id !== "" && !previous.has(id));
        const ids = next.get(control) ?? new Set<string>();
        const description = [...new Set([...own, ...ids])].join(" ");
        if (description !== (control.getAttribute("aria-describedby") ?? "")) {
          if (description === "") control.removeAttribute("aria-describedby");
          else control.setAttribute("aria-describedby", description);
        }
        if (ids.size === 0) linked.delete(control);
        else linked.set(control, ids);
      }
    };
    connect();
    const observer = new MutationObserver(connect);
    observer.observe(page, {
      childList: true,
      subtree: true,
      // Selectors update their own description when their value changes; append the helper again.
      attributes: true,
      attributeFilter: ["aria-describedby"],
    });
    return () => {
      observer.disconnect();
    };
  }, [prefix, root]);
}
