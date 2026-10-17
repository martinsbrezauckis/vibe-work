/**
 * Palette entries of the user's project groups: "New project" and "Move the open session to a
 * project…" (the keyboard path next to drag and drop and the row menu). Both open the same list UI the pointer uses
 * (the name field, the move menu), so the palette can do nothing the list cannot.
 * Depends on: AppContext types, state/projectGroups. Called by: useSessionWiring.
 * Invariants: the list is made visible first (the Projects overview closes, a narrow window opens its drawer); the
 * move entry acts on the selected session only and does nothing without one.
 */
import { term } from "../../vocabulary";
import { computeLayout } from "../../state/panelLayout";
import type { App } from "../shell/AppContext";

/** Where the move menu opens: under the selected row, else near the top of the list. */
function anchor(sessionId: string): { x: number; y: number } {
  const row = document.querySelector(`[data-session-id="${CSS.escape(sessionId)}"]`);
  const box = row?.getBoundingClientRect();
  return box === undefined || box.height === 0
    ? { x: 80, y: 120 }
    : { x: box.left + 24, y: box.bottom };
}

/** Registers the entries; returns the function that removes them. */
export function registerProjectCommands(app: App): () => void {
  const groups = app.projects?.groups;
  if (groups === undefined) return () => undefined;
  const showList = () => {
    app.projects?.close();
    if (app.shell.store.get().filter !== "") app.shell.actions.setFilter("");
    const layout = computeLayout(app.shell.store.get(), window.innerWidth);
    if (layout.drawer) app.shell.actions.setDrawer(true);
    else if (!layout.listVisible) app.shell.actions.showList();
  };
  const removers = [
    app.commands.register({
      id: "projects.new",
      label: "New project",
      section: term("workNames", "projectTab").name,
      keywords: "create project group folder",
      run: () => {
        showList();
        groups.setCreating(true);
      },
    }),
    app.commands.register({
      id: "projects.moveSession",
      label: "Move the open session to a project...",
      section: term("workNames", "projectTab").name,
      keywords: "file group project move session",
      run: () => {
        const selected = app.shell.store.get().selected;
        if (selected === null || groups.store.get().data === null) return;
        showList();
        groups.openMover([selected], anchor(selected));
      },
    }),
  ];
  return () => {
    for (const remove of removers) remove();
  };
}
