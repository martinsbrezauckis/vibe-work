/**
 * The window frame: title bar, icon rail, list column, content, on-demand side panel, status bar,
 * the Settings page layer and the overlays (command palette, toasts). Layout is computed by `computeLayout`
 * (`computeLayout`) and applied as classes and one CSS variable (`--pw`).
 * Depends on: every shell component, hooks. Called by: main.tsx (inside `AppProvider`).
 * Invariants: components read the stores only; no layout arithmetic here beyond applying the
 * result; the panel is always in the DOM (closed it is `visibility: hidden`, so the grid
 * transition can animate). While the Projects overview is open it takes the list
 * column and the side panel's room (they belong to the selected session; the panel's tab body,
 * with any native view in it, is unmounted meanwhile, see SidePanel).
 */
import { type CSSProperties, useState } from "react";

import { CommandPalette } from "./CommandPalette";
import { Content } from "./Content";
import { Rail } from "./Rail";
import { SessionList } from "./SessionList";
import { SidePanel } from "./SidePanel";
import { StatusBar } from "./StatusBar";
import { TitleBar } from "./TitleBar";
import { Toasts } from "./Toasts";
import { useProjects } from "../projects/hooks";
import { DeleteProjectDialog } from "../sessions/DeleteProjectDialog";
import { UpdateNotice } from "../updates/UpdateNotice";
import { VibeUpdateDialog } from "../updates/VibeUpdate";
import { useLayout, useShellState } from "./hooks";
import { useShellWiring } from "./useShellWiring";

/** The shell. */
export function Shell() {
  useShellWiring();
  const layout = useLayout();
  const [dragging, setDragging] = useState(false);
  const overview = useProjects((s) => s.open);
  // The Settings page keeps the list column (its categories) even over the Projects overview.
  const settings = useShellState((s) => s.settings.open);
  const classes = [
    "body",
    layout.panelOpen && !overview ? "has-panel" : "",
    layout.listVisible && (!overview || settings) ? "" : "no-col",
    overview && !settings ? "is-overview" : "",
    settings ? "is-settings" : "",
    layout.maximized ? "panel-max" : "",
    layout.wide && layout.panelOpen ? "is-wide" : "",
    dragging ? "dragging" : "",
  ]
    .filter((c) => c !== "")
    .join(" ");
  return (
    <>
      <div className="app">
        <TitleBar />
        <div
          className={classes}
          id="body"
          style={{ "--pw": `${String(layout.panelWidth)}px` } as CSSProperties}
        >
          <Rail />
          <SessionList />
          <Content />
          <SidePanel onDragging={setDragging} />
        </div>
        <StatusBar>
          <Toasts />
        </StatusBar>
      </div>
      <CommandPalette />
      <DeleteProjectDialog />
      <UpdateNotice />
      <VibeUpdateDialog />
    </>
  );
}
