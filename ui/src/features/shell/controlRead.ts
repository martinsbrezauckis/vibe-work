/** Reads main-window controls for Debug API outcome checks. Uses secret-neutral queries; input values marked secret never leave the window.
 * Depends on the shell app and UI-control registry. Native pages and editor shadow controls retain their typed actions. */
import type { App } from "./AppContext";
import { bodyObject } from "../../state/routeInput";
import { UiControlError } from "../../state/uiControl";
import { elementVisible } from "./elementVisibility";
import { queryNeutral } from "./secretSafety";

const SECRET = '[data-secret], input[type="password"]';

function safeText(element: HTMLElement): string {
  if (element.matches(SECRET)) return "[secret field]";
  const copy = element.cloneNode(true) as HTMLElement;
  for (const secret of copy.querySelectorAll(SECRET)) secret.replaceWith("[secret field]");
  for (const hidden of copy.querySelectorAll("script,style,[hidden],[aria-hidden=true]"))
    hidden.remove();
  return copy.textContent.trim();
}

/** Registers a pure read of rendered controls, or the visible elements a selector names. */
export function registerControlRead(app: App): () => void {
  return app.uiControl.register(
    "controls",
    (body) => {
      const input = bodyObject(body);
      const selector = input["selector"] ?? "[data-testid]";
      const includeHidden = input["includeHidden"] ?? false;
      if (typeof includeHidden !== "boolean")
        throw new UiControlError("invalidParams", "includeHidden must be a boolean");
      if (typeof selector !== "string" || selector === "")
        throw new UiControlError("invalidParams", "selector must be a non-empty CSS selector");
      return queryNeutral(() => {
        let elements: HTMLElement[];
        try {
          elements =
            selector.trim() === ":focus"
              ? document.activeElement instanceof HTMLElement
                ? [document.activeElement]
                : []
              : [...document.querySelectorAll<HTMLElement>(selector)];
        } catch {
          throw new UiControlError("invalidParams", "selector is not a valid CSS selector");
        }
        return {
          controls: elements
            .filter((element) => includeHidden || elementVisible(element))
            .map((element) => {
              const secret = element.matches(SECRET);
              const field =
                element instanceof HTMLInputElement ||
                element instanceof HTMLTextAreaElement ||
                element instanceof HTMLSelectElement;
              return {
                testId: element.dataset["testid"] ?? null,
                itemId: element.dataset["itemId"] ?? null,
                tag: element.tagName.toLowerCase(),
                visible: elementVisible(element),
                role: element.getAttribute("role"),
                label: element.getAttribute("aria-label") ?? element.getAttribute("title"),
                text: safeText(element),
                value: secret
                  ? null
                  : field
                    ? element.value
                    : element.getAttribute("aria-valuenow"),
                secret,
                checked: element instanceof HTMLInputElement ? element.checked : null,
                ariaChecked: element.getAttribute("aria-checked"),
                labelledBy: element.getAttribute("aria-labelledby"),
                describedBy: element.getAttribute("aria-describedby"),
                live: element.getAttribute("aria-live"),
                atomic: element.getAttribute("aria-atomic"),
                focused: document.activeElement === element,
                pressed: element.getAttribute("aria-pressed"),
                expanded: element.getAttribute("aria-expanded"),
                selected: element.getAttribute("aria-selected"),
                disabled: element.matches(":disabled,[aria-disabled=true]"),
              };
            }),
        };
      });
    },
    () => true,
  );
}
