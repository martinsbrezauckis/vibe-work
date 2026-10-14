/**
 * The `ui/click` and `ui/key` routes (the Debug API controls the whole app as
 * the user): a click on any element, or a key press, exactly where a user's would land, for every control that
 * has no route of its own (a menu button, a row's actions, Choose folder). The Rust side lists the same names
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES).
 * Depends on: state/uiControl, AppContext types. Called by: useShellWiring.
 * Invariants:
 * - The click is the element's own `click()` (the same handlers a mouse click runs); a disabled or hidden
 *   element is refused (`denied`), never clicked, so a route can do only what a user can.
 * - Commands a click or key sends while the request is served carry the Debug API origin (`uiControl` marks).
 * - The click commits React effects before uiControl drains their commands; native folder pickers keep running.
 * - Selectors are matched with every secret value emptied (`secretSafety.ts`): no yes/no oracle.
 */
import { flushSync } from "react-dom";

import { UiControlError } from "../../state/uiControl";
import type { App } from "./AppContext";
import { queryNeutral } from "./secretSafety";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** The `index`-th element matching `selector` (CSS), refused when it is missing, hidden or disabled. */
function target(selector: unknown, index: unknown): HTMLElement {
  if (typeof selector !== "string" || selector === "")
    throw new UiControlError("invalidParams", "selector must be a non-empty CSS selector");
  // Decided on the result with every secret value emptied (`secretSafety.ts`): no value oracle.
  const all = queryNeutral(() => {
    try {
      return [...document.querySelectorAll<HTMLElement>(selector)];
    } catch {
      throw new UiControlError("invalidParams", "that is not a valid CSS selector");
    }
  });
  const at = typeof index === "number" && Number.isInteger(index) && index >= 0 ? index : 0;
  const el = all[at];
  if (el === undefined) throw new UiControlError("notFound", "no element matches that selector");
  const rect = el.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1 || getComputedStyle(el).visibility === "hidden")
    throw new UiControlError("denied", "the element is not visible");
  const disabled = el instanceof HTMLButtonElement && el.disabled;
  if (disabled || el.getAttribute("aria-disabled") === "true")
    throw new UiControlError("denied", "the element is disabled");
  return el;
}

/** Registers `click` and `key`; returns the function that removes them. */
export function registerInputRoutes(app: App): () => void {
  const off = [
    app.uiControl.register("click", (body) => {
      const { selector, index } = asObject(body);
      const el = target(selector, index);
      el.focus();
      // Commit click effects before uiControl drains their commands and releases the request's capability.
      flushSync(() => {
        el.click();
      });
      // Only the label or the tag: never the element's text, which could be a secret field's neighbourhood.
      return { clicked: el.getAttribute("aria-label") ?? el.tagName.toLowerCase() };
    }),
    app.uiControl.register("key", (body) => {
      const { key, selector, index, ctrl, shift, alt, meta } = asObject(body);
      if (typeof key !== "string" || key === "")
        throw new UiControlError("invalidParams", "key must be a key name such as Escape or k");
      const el =
        selector === undefined
          ? (document.activeElement ?? document.body)
          : target(selector, index);
      if (el instanceof HTMLElement && selector !== undefined) el.focus();
      const init: KeyboardEventInit = {
        key,
        bubbles: true,
        cancelable: true,
        ctrlKey: ctrl === true,
        shiftKey: shift === true,
        altKey: alt === true,
        metaKey: meta === true,
      };
      el.dispatchEvent(new KeyboardEvent("keydown", init));
      el.dispatchEvent(new KeyboardEvent("keyup", init));
      return { key };
    }),
  ];
  return () => {
    for (const remove of off) remove();
  };
}
