/**
 * The `ui/click`, `ui/key` and `ui/menu` routes (the Debug API controls the whole app as
 * the user): a click on any element, a key press or a right click, exactly where a user's would land, for every
 * control that has no route of its own (a menu button, a row's actions, Choose folder), and the open action menu
 * (read it, choose an item as a click does). The Rust side lists the same names
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES).
 * Depends on: state/uiControl, state/openMenu, AppContext types. Called by: useShellWiring.
 * Invariants:
 * - The click is the element's own `click()` (the same handlers a mouse click runs); a disabled or hidden
 *   element is refused (`denied`), never clicked, so a route can do only what a user can.
 * - Commands a click or key sends while the request is served carry the Debug API origin (`uiControl` marks).
 * - The click commits React effects before uiControl drains their commands; native folder pickers keep running.
 * - Selectors are matched with every secret value emptied (`secretSafety.ts`): no yes/no oracle.
 */
import { flushSync } from "react-dom";

import { openMenu } from "../../state/openMenu";
import { parseKeys } from "../../state/shortcuts";
import { UiControlError } from "../../state/uiControl";
import type { App } from "./AppContext";
import { elementVisible } from "./elementVisibility";
import { queryNeutral } from "./secretSafety";

function asObject(body: unknown): Record<string, unknown> {
  if (typeof body === "object" && body !== null && !Array.isArray(body))
    return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** Use the shortcut registry's physical key names for keyboard actions as well as character input. */
function physicalCode(key: string): string {
  try {
    return parseKeys(key === " " ? "Space" : key).code;
  } catch {
    return key;
  }
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
  if (!elementVisible(el)) throw new UiControlError("denied", "the element is not visible");
  const disabled = el instanceof HTMLButtonElement && el.disabled;
  if (disabled || el.getAttribute("aria-disabled") === "true")
    throw new UiControlError("denied", "the element is disabled");
  return el;
}

const INTERACTIVE =
  "button,a,input,select,textarea,summary,[role='button'],[role='tab'],[role='menuitem'],[role='option'],[role='switch'],[role='combobox']";

function triggerId(element: Element): string | null {
  const control = element.closest(INTERACTIVE);
  return (control ?? element.closest("[data-testid]"))?.getAttribute("data-testid") ?? null;
}

/** Registers input and read-only trace routes; returns the function that removes them. */
export function registerInputRoutes(app: App): () => void {
  const onClick = (event: MouseEvent) => {
    app.uiControl.clickTrigger(event.target instanceof Element ? triggerId(event.target) : null);
  };
  document.addEventListener("click", onClick, true);
  const off = [
    app.uiControl.register("trace", (body) => {
      const { since = 0 } = asObject(body);
      if (typeof since !== "number" || !Number.isSafeInteger(since) || since < 0)
        throw new UiControlError("invalidParams", "since must be a non-negative safe integer");
      return app.uiControl.trace.read(since);
    }),
    app.uiControl.register("click", (body) => {
      const { selector, index } = asObject(body);
      const el = target(selector, index);
      app.uiControl.clickTrigger(triggerId(el));
      el.focus();
      // Commit click effects before uiControl drains their commands and releases the request's capability.
      flushSync(() => {
        el.click();
      });
      // Only the label or the tag: never the element's text, which could be a secret field's neighbourhood.
      return { clicked: el.getAttribute("aria-label") ?? el.tagName.toLowerCase() };
    }),
    app.uiControl.register("key", (body) => {
      const { key, code, selector, index, ctrl, shift, alt, meta } = asObject(body);
      if (typeof key !== "string" || key === "")
        throw new UiControlError("invalidParams", "key must be a key name such as Escape or k");
      if (code !== undefined && (typeof code !== "string" || code === ""))
        throw new UiControlError("invalidParams", "code must be a physical key name such as KeyK");
      const el =
        selector === undefined
          ? (document.activeElement ?? document.body)
          : target(selector, index);
      if (el instanceof HTMLElement && selector !== undefined) el.focus();
      const init: KeyboardEventInit = {
        key,
        code: typeof code === "string" ? code : physicalCode(key),
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
    app.uiControl.register(
      "menu",
      (body) => {
        const { action = "state", selector, index, item } = asObject(body ?? {});
        const shown = () => {
          const menu = openMenu();
          return menu === null
            ? { open: false }
            : {
                open: true,
                testId: menu.testId,
                label: menu.label,
                heading: menu.heading,
                items: menu.items,
              };
        };
        if (action === "state") return shown();
        if (action === "open") {
          // A right click in the middle of the element: the same handler the mouse runs.
          const el = target(selector, index);
          const box = el.getBoundingClientRect();
          flushSync(() => {
            el.dispatchEvent(
              new MouseEvent("contextmenu", {
                bubbles: true,
                cancelable: true,
                button: 2,
                clientX: box.left + Math.min(box.width / 2, 40),
                clientY: box.top + Math.min(box.height / 2, 12),
              }),
            );
          });
          return shown();
        }
        const menu = openMenu();
        if (menu === null) throw new UiControlError("notFound", "no menu is open");
        if (action === "close") {
          flushSync(() => {
            menu.close();
          });
          return shown();
        }
        if (action !== "choose")
          throw new UiControlError("invalidParams", "action must be state, open, choose or close");
        if (typeof item !== "string")
          throw new UiControlError("invalidParams", "item must be the id of a menu item");
        const entry = menu.items.find((candidate) => candidate.id === item);
        if (entry === undefined)
          throw new UiControlError("notFound", "the open menu has no such item");
        if (entry.disabled) throw new UiControlError("denied", "that item is disabled");
        flushSync(() => {
          menu.choose(item);
        });
        return { chosen: item };
      },
      (body) => {
        const action = (body as { action?: unknown } | null)?.action;
        return action === undefined || action === "state";
      },
    ),
  ];
  return () => {
    document.removeEventListener("click", onClick, true);
    for (const remove of off) remove();
  };
}
