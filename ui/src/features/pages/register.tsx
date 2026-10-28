/**
 * Plugs the project handbook into the app: creates the controller, registers the Project tab body
 * and the Plan tab body (the plan checklist) over the placeholders, sends the app's theme to the viewer,
 * the palette entries, the "Open the plan" shortcut (Mod+Shift+P: the full plan page), the `ui/project` route (and
 * the compatibility `ui/pages` route), the `ui/plan` route, and the page opener the Projects
 * overview uses.
 * Depends on: store.ts, PagesTab, pagesRoutes.ts, AppContext types. Called by: main.tsx (after the placeholders), the
 * UI test rig.
 * Invariants: without a core nothing is registered (the placeholder stays); the returned function undoes every
 * registration, hides the viewer and stops following events.
 */
import { term } from "../../vocabulary";
import type { App } from "../shell/AppContext";
import { PagesTab } from "./PagesTab";
import { PlanChecklist } from "./PlanChecklist";
import { registerPagesRoute, registerPlanRoute } from "./pagesRoutes";
import { createPages, type Pages } from "./store";

/** Opens the Project tab on the current plan of the selected session. */
function openPlan(app: App, pages: Pages): void {
  const sessionId = app.shell.store.get().selected;
  if (sessionId === null) {
    app.shell.actions.openTab("project");
    return;
  }
  app.shell.actions.openTab("project");
  void pages.openPlan(sessionId);
}

/** Registers the Project tab on `app`; returns the function that removes it. */
export function registerPagesTab(app: App): () => void {
  const core = app.core;
  if (core === null) return () => undefined;
  const pages = createPages(core);
  // the viewer passes the app's theme to every page (Format 1.2 §6.4); sent now and on every change.
  let theme = app.shell.store.get().theme;
  void pages.setTheme(theme);
  const stopTheme = app.shell.store.subscribe(() => {
    const next = app.shell.store.get().theme;
    if (next === theme) return;
    theme = next;
    void pages.setTheme(next);
  });
  /** The plan checklist's click: the step of the full plan page, in the Project tab. */
  const openStep = (sessionId: string, path: string, step: string) => {
    app.shell.actions.openTab("project");
    void pages.openAt(sessionId, path, step);
  };
  const removers = [
    stopTheme,
    app.panelTabs.register("project", () => <PagesTab pages={pages} />),
    app.panelTabs.register("plan", () => <PlanChecklist pages={pages} openStep={openStep} />),
    registerPlanRoute(app, pages, openStep),
    app.commands.register({
      id: "project.plan",
      label: "Open the plan",
      section: term("workNames", "projectTab").name,
      keys: "Mod+Shift+P",
      keywords: "plan progress steps handbook",
      run: () => {
        openPlan(app, pages);
      },
    }),
    app.commands.register({
      id: "project.handbook",
      label: term("workNames", "handbook").tooltip,
      section: term("workNames", "projectTab").name,
      keywords: "pages hub handbook plans decisions requests reports",
      run: () => {
        const sessionId = app.shell.store.get().selected;
        app.shell.actions.openTab("project");
        if (sessionId !== null) pages.showHub(sessionId);
      },
    }),
    app.shortcuts.register({
      id: "project.plan",
      keys: "Mod+Shift+P",
      description: "Open the plan of this session's project",
      run: () => {
        openPlan(app, pages);
      },
    }),
    registerPagesRoute(app, pages),
    // The Projects overview opens a project's handbook (or one page) in a session's Project tab through this.
    app.projects?.setOpener((sessionId, path) => {
      app.shell.actions.openTab("project");
      if (path === null) pages.showHub(sessionId);
      else void pages.open(sessionId, path);
    }) ?? (() => undefined),
  ];
  return () => {
    for (const remove of removers) remove();
    pages.layout(null);
    pages.stop();
  };
}
