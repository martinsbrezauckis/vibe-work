/** Applies remembered display preferences and zoom shortcuts; installed and removed by the shell. */
import { TEXT_SIZES } from "../../state/accessibility";
import type { App } from "./AppContext";

export function startAccessibility(app: App): () => void {
  const root = document.documentElement;
  const contrast =
    typeof window.matchMedia === "function" ? window.matchMedia("(prefers-contrast: more)") : null;
  const theme =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: light)")
      : null;
  const apply = () => {
    const s = app.accessibility.store.get();
    root.dataset["contrast"] = s.contrast === "more" || contrast?.matches ? "more" : "system";
    root.dataset["motion"] = s.motion;
    root.style.zoom = String(s.textSize / 100);
    root.style.setProperty("--window-height", `${String(10000 / s.textSize)}vh`);
    window.dispatchEvent(new Event("resize"));
  };
  const followTheme = () => {
    app.shell.actions.followSystemTheme(theme?.matches === true ? "light" : "dark");
  };
  const key = (event: KeyboardEvent) => {
    if (!(event.ctrlKey || event.metaKey) || event.altKey || event.isComposing) return;
    const index = TEXT_SIZES.findIndex((size) => size === app.accessibility.store.get().textSize);
    const next =
      event.key === "0"
        ? 100
        : ["+", "="].includes(event.key)
          ? TEXT_SIZES[Math.min(index + 1, TEXT_SIZES.length - 1)]
          : event.key === "-"
            ? TEXT_SIZES[Math.max(index - 1, 0)]
            : undefined;
    if (next === undefined) return;
    event.preventDefault();
    app.accessibility.update({ textSize: next });
  };
  apply();
  followTheme();
  const stop = app.accessibility.store.subscribe(apply);
  contrast?.addEventListener("change", apply);
  theme?.addEventListener("change", followTheme);
  window.addEventListener("keydown", key, true);
  return () => {
    stop();
    contrast?.removeEventListener("change", apply);
    theme?.removeEventListener("change", followTheme);
    window.removeEventListener("keydown", key, true);
    root.style.removeProperty("zoom");
    root.style.removeProperty("--window-height");
    delete root.dataset["contrast"];
    delete root.dataset["motion"];
  };
}
