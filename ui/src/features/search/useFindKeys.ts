/**
 * Ctrl F / Cmd F in the work view ("scope follows where you search"): in the File tab it opens find
 * in that file, anywhere else in the work view it opens find in the open conversation. Settings has its own Ctrl F
 * (its search field); the global search stays on Ctrl K.
 * Depends on: AppContext. Called by: shell/Content.
 * Invariants: does nothing while a page covers the work view (Settings, Projects, Tools) or the palette is open, so
 * the page's own Ctrl F wins; the browser's own find never opens instead (default prevented only when handled).
 */
import { useEffect } from "react";

import { useApp } from "../shell/AppContext";

/** Installs the key handler while mounted. */
export function useFindKeys(covered: boolean): void {
  const app = useApp();
  useEffect(() => {
    if (covered) return;
    const onKey = (e: KeyboardEvent) => {
      if (
        e.defaultPrevented ||
        e
          .composedPath()
          .some((target) => target instanceof Element && target.matches("[data-code-editor]"))
      )
        return;
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== "f")
        return;
      const shell = app.shell.store.get();
      if (shell.overlay !== null || shell.settings.open) return;
      const sessionId = shell.selected;
      if (sessionId === null) return;
      e.preventDefault();
      const active = document.activeElement;
      if (active instanceof Element && active.closest(".file-tab") !== null)
        app.find.actions.openFile(sessionId);
      else app.find.actions.open(sessionId);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [app, covered]);
}
