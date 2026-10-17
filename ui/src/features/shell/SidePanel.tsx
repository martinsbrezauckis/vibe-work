/**
 * The on-demand side panel: tabs (Project, Browser, Changes, Terminal), maximize, close, and a
 * resize grip. Closed by default; the width of each tab kind is remembered once dragged.
 * Depends on: AppContext, hooks, components/Icon, state/shellState, state/panelLayout.
 * Called by: Shell. Tab bodies come from the `panelTabs` registry.
 * Accessibility: the grip is a keyboard-operable separator (arrow keys resize by 24 px); each tab
 * and its close button are separate controls (no interactive element inside another).
 */
import { type KeyboardEvent, type PointerEvent, useLayoutEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { IconButton } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import { formatKeys } from "../../state/shortcuts";
import { CHAT_MIN, PANEL_MIN, RAIL_W } from "../../state/panelLayout";
import { TAB_KINDS, TAB_META, panelOf } from "../../state/shellState";
import { useProjects } from "../projects/hooks";
import { useApp } from "./AppContext";
import { TAB_ICON } from "./tabIcons";
import { useLayout, useShellState } from "./hooks";

const KEY_STEP = 24;

/** The side panel. `onDragging` tells the grid to drop its transition while the grip moves. */
export function SidePanel({ onDragging }: { onDragging: (dragging: boolean) => void }) {
  const { shell, panelTabs } = useApp();
  const panel = useShellState(panelOf);
  const layout = useLayout();
  const drag = useRef(false);
  // The Projects overview covers the panel: its tab body (and a native view in it) is unmounted.
  const overview = useProjects((s) => s.open);
  // The Settings page covers the panel the same way.
  const settings = useShellState((s) => s.settings.open);
  const covered = overview || settings;
  const active = panel.open && !covered ? panel.active : null;
  const hidden = useStore(panelTabs.hidden, (h) => h);
  const remaining = TAB_KINDS.find((k) => !panel.tabs.includes(k) && !hidden.includes(k));

  const widthFor = (clientX: number) =>
    Math.max(
      PANEL_MIN,
      Math.min(window.innerWidth - clientX, window.innerWidth - RAIL_W - CHAT_MIN),
    );
  const onDown = (event: PointerEvent<HTMLDivElement>) => {
    if (active === null) return;
    drag.current = true;
    event.currentTarget.setPointerCapture(event.pointerId);
    onDragging(true);
  };
  const onMove = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current && active !== null) shell.actions.setWidth(active, widthFor(event.clientX));
  };
  const onUp = () => {
    drag.current = false;
    onDragging(false);
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (active === null) return;
    const delta = event.key === "ArrowLeft" ? KEY_STEP : event.key === "ArrowRight" ? -KEY_STEP : 0;
    if (delta === 0) return;
    event.preventDefault();
    shell.actions.setWidth(active, Math.max(PANEL_MIN, layout.panelWidth + delta));
  };

  // A tab body is a render function that returns an element; hooks live in that element's
  // component, never in the body function itself (see state/panelTabs.ts).
  const body = active === null ? null : (panelTabs.body(active)?.() ?? null);
  const tabStrip = useRef<HTMLDivElement>(null);
  const [pill, setPill] = useState<{ x: number; w: number } | null>(null);
  useLayoutEffect(() => {
    const on = tabStrip.current?.querySelector<HTMLElement>('.ptab-group[data-selected="true"]');
    const next = on === null || on === undefined ? null : { x: on.offsetLeft, w: on.offsetWidth };
    setPill((prev) => (prev?.x === next?.x && prev?.w === next?.w ? prev : next));
  }, [active, panel.tabs, layout.panelWidth]);
  return (
    <section
      className="panel"
      id="panel"
      aria-label="Side panel"
      aria-hidden={!layout.panelOpen || covered}
    >
      <div
        data-testid="shell.side-panel-grip"
        className="grip"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the side panel (Left or Right arrow)"
        title="Resize the side panel (Left or Right arrow)"
        aria-valuenow={layout.panelWidth}
        aria-valuemin={PANEL_MIN}
        tabIndex={layout.panelOpen && !covered ? 0 : -1}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onKeyDown={onKey}
      />
      <div
        data-testid="shell.side-panel-ptabs"
        className="ptabs"
        role="tablist"
        aria-label="Side panel tabs"
        ref={tabStrip}
      >
        {/* the selected tab's pill slides to the new tab (200 ms) instead of jumping. */}
        {pill !== null && (
          <span
            className="ptab-pill"
            aria-hidden="true"
            style={{ width: pill.w, transform: `translateX(${String(pill.x)}px)` }}
          />
        )}
        {panel.tabs.map((kind) => (
          <span
            data-item-id={kind}
            key={kind}
            className="ptab-group"
            data-selected={kind === active}
          >
            <button
              data-testid="shell.side-panel-open-tab"
              type="button"
              role="tab"
              className="ptab"
              aria-selected={kind === active}
              data-tab={kind}
              title={`Show ${TAB_META[kind].label} tab${kind === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
              aria-label={`Show ${TAB_META[kind].label} tab${kind === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
              onClick={() => {
                shell.actions.openTab(kind);
              }}
            >
              <Icon name={TAB_ICON[kind]} size={14} />
              {TAB_META[kind].label}
            </button>
            <button
              data-testid="shell.side-panel-close-tab"
              type="button"
              className="ptab-x"
              aria-label={`Close ${TAB_META[kind].label} tab`}
              title={`Close ${TAB_META[kind].label} tab`}
              onClick={() => {
                shell.actions.closeTab(kind);
              }}
            >
              <Icon name="x" size={12} />
            </button>
          </span>
        ))}
        <span className="end">
          {remaining !== undefined && (
            <IconButton
              data-testid="shell.side-panel-open-tab-2"
              icon="plus"
              aria-label={`Open ${TAB_META[remaining].label} tab${remaining === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
              title={`Open ${TAB_META[remaining].label} tab${remaining === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
              onClick={() => {
                shell.actions.openTab(remaining);
              }}
            />
          )}
          <IconButton
            data-testid="shell.side-panel-maximize-2"
            icon="maximize-2"
            aria-label={layout.maximized ? "Restore side panel" : "Maximize side panel"}
            title={layout.maximized ? "Restore side panel" : "Maximize side panel"}
            aria-pressed={layout.maximized}
            onClick={shell.actions.toggleMaximized}
          />
          <IconButton
            data-testid="shell.side-panel-x"
            icon="x"
            aria-label="Close the panel"
            title="Close the panel"
            onClick={shell.actions.closePanel}
          />
        </span>
      </div>
      <div data-testid="shell.side-panel-pbody" className="pbody" role="tabpanel">
        {body ?? <div className="empty">Nothing to show.</div>}
      </div>
    </section>
  );
}
