/**
 * The `ui/setup` route: reads what the key screens show, for the Debug API and end-to-end tests. READ-ONLY on purpose: a route that typed a password or pressed a button
 * would be a second way to change the key outside the app's own screens. (Since a command
 * a route triggers goes out with the Debug API origin, so a key change would ask the native
 * confirmation; the route still only reads: key changes from the Debug API use the `keys` command.)
 * Depends on: AppContext types, state/keyView, state/uiControl. Called by: useShellWiring.
 * Invariants: no secret and no path in the answer (statuses, counts, reason words only); the route
 * exists in every run and says `available: false` when there is no key service.
 */
import { stepOf } from "../../state/keyView";
import type { App } from "../shell/AppContext";

/** Summarizes only known, non-secret setup controls from the rendered screen. */
function setupDom() {
  const screen = document.querySelector('[data-testid="setup-screen"]');
  if (screen === null) return null;
  const controls = Array.from(screen.querySelectorAll("[data-testid]"));
  // Shared Selector options are portalled; only include menus owned by setup triggers.
  for (const trigger of screen.querySelectorAll('[role="combobox"][aria-controls]')) {
    const menu = document.getElementById(trigger.getAttribute("aria-controls") ?? "");
    if (menu?.getAttribute("data-selector-trigger") === trigger.id)
      controls.push(...menu.querySelectorAll("[data-testid]"));
  }
  return {
    step: screen.getAttribute("data-step"),
    phase: screen.querySelector('[data-testid="setup-result"]')?.getAttribute("data-phase") ?? null,
    headings: Array.from(screen.querySelectorAll("h2, h3"), (heading) =>
      heading.textContent.trim(),
    ),
    controls: controls.map((element) => ({
      tag: element.tagName.toLowerCase(),
      testId: element.getAttribute("data-testid"),
      type: element.getAttribute("type"),
      disabled:
        element instanceof HTMLButtonElement || element instanceof HTMLInputElement
          ? element.disabled
          : null,
      checked: element instanceof HTMLInputElement ? element.checked : null,
      // Screenshot drivers attest the blank key field without retrieving a value.
      empty:
        element instanceof HTMLInputElement &&
        element.getAttribute("data-testid") === "setup.key-input"
          ? element.value === ""
          : null,
      ariaPressed: element.getAttribute("aria-pressed"),
      ariaSelected: element.getAttribute("aria-selected"),
      ariaExpanded: element.getAttribute("aria-expanded"),
      label: element instanceof HTMLButtonElement ? element.textContent.trim() : null,
    })),
  };
}

/** Registers the route; returns the function that removes it. */
export function registerSetupRoutes(app: App): () => void {
  return app.uiControl.register("setup", () => {
    const keys = app.keys;
    // the Vibe install's phase (the install itself is the `vibe` command).
    const vibe = app.vibe?.store.get().view?.phase.state ?? null;
    if (keys === null) return { available: false, vibe, dom: setupDom() };
    const s = keys.store.get();
    return {
      available: s.availability === "ready",
      covers: keys.covers(),
      step: stepOf(s.status).kind,
      vaultState: s.status?.vaultState ?? null,
      vaultKind: s.status?.vaultKind ?? null,
      keyState: s.status?.keyState ?? null,
      rememberedDevice: s.status?.rememberedDevice ?? false,
      need: s.need,
      busy: s.busy,
      test: s.test.phase === "failed" ? { phase: "failed", reason: s.test.reason } : s.test,
      problem: s.problem?.reason ?? null,
      savedKeys: s.candidates.length,
      vibeKeys: s.vibeKeys.map((k) => k.source),
      resultPending: s.resultPending,
      dismissed: s.dismissed,
      vibe,
      dom: setupDom(),
    };
  });
}
