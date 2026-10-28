/**
 * The on-demand side panel: tabs (Project, Browser, Changes, Terminal), maximize, close, and a
 * resize grip. Closed by default; the width of each tab kind is remembered once dragged.
 * Depends on: AppContext, hooks, components/Icon, state/shellState, state/panelLayout.
 * Called by: Shell. Tab bodies come from the `panelTabs` registry.
 * Accessibility: the grip is a keyboard-operable separator (arrow keys resize by 24 px); each tab
 * and its close button are separate controls (no interactive element inside another).
 */
import { FilePanelTabs } from "./FilePanelTabs";
import { Tabs, Tab, TabClose } from "../../components/Tabs";
import { type KeyboardEvent, type PointerEvent, useLayoutEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { IconButton } from "../../components/Controls";
import type { FileWorkspace } from "../../state/fileWorkspace";
import { createStore, useStore } from "../../state/createStore";
import { formatKeys } from "../../state/shortcuts";
import { CHAT_MIN, PANEL_MIN, RAIL_W } from "../../state/panelLayout";
import { TAB_KINDS, TAB_META, panelOf } from "../../state/shellState";
import { useProjects } from "../projects/hooks";
import { useApp } from "./AppContext";
import { TAB_ICON } from "./tabIcons";
import { useLayout, useShellState } from "./hooks";

const KEY_STEP = 24;
const NO_DOCUMENTS = createStore<Readonly<Record<string, FileWorkspace>>>({});

/** The side panel. `onDragging` tells the grid to drop its transition while the grip moves. */
export function SidePanel({ onDragging }: { onDragging: (dragging: boolean) => void }) {
  const { shell, panelTabs, files } = useApp();
  const sessionId = useShellState((s) => s.selected);
  const selectedFile = useStore(
    files?.workspace.store ?? NO_DOCUMENTS,
    (s) => s[sessionId ?? ""]?.active ?? null,
  );
  const separate = useShellState((s) => s.openFilesIn === "tabs");
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
    if (drag.current && active !== null) {
      if (layout.editorSplit) shell.actions.setEditorLayout({ width: event.clientX - RAIL_W - 48 });
      else shell.actions.setWidth(active, widthFor(event.clientX));
    }
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
    if (layout.editorSplit) shell.actions.setEditorLayout({ width: layout.slotWidth - delta });
    else shell.actions.setWidth(active, Math.max(PANEL_MIN, layout.panelWidth + delta));
  };

  // A tab body is a render function that returns an element; hooks live in that element's
  // component, never in the body function itself (see state/panelTabs.ts).
  const body = active === null ? null : (panelTabs.body(active)?.() ?? null);
  const tabStrip = useRef<HTMLDivElement>(null);
  const [pill, setPill] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const on = tabStrip.current?.querySelector<HTMLElement>('.ptab-group[data-selected="true"]');
    // The pill covers the selected tab where the bar centres it (the bar is as tall as the chat header).
    const next =
      on === null || on === undefined
        ? null
        : { x: on.offsetLeft, y: on.offsetTop, w: on.offsetWidth, h: on.offsetHeight };
    setPill((prev) =>
      prev?.x === next?.x && prev?.y === next?.y && prev?.w === next?.w && prev?.h === next?.h
        ? prev
        : next,
    );
  }, [active, panel.tabs, layout.panelWidth, selectedFile]);
  useLayoutEffect(() => {
    const element = tabStrip.current;
    if (!element || !files || sessionId === null) return;
    const fit = () => {
      const fixed = [
        ...element.querySelectorAll<HTMLElement>(".ptab-group:not(.file-panel-tab), .end"),
      ].reduce((sum, e) => sum + e.offsetWidth + 2, 0);
      // A file tab shrinks to 92 px (short name, full path on hover); the +N button needs 44 px.
      files.workspace.capacity(
        sessionId,
        Math.max(1, Math.floor((element.clientWidth - fixed - 44) / 92)),
      );
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [files, sessionId, panel.tabs]);
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
      <Tabs
        data-testid="shell.side-panel-ptabs"
        variant="pill"

        aria-label="Side panel tabs"
        ref={tabStrip}
      >
        {/* the selected tab's pill slides to the new tab (200 ms) instead of jumping. */}
        {pill !== null && (
          <span
            className="ptab-pill"
            aria-hidden="true"
            style={{
              width: pill.w,
              height: pill.h,
              transform: `translate(${String(pill.x)}px, ${String(pill.y)}px)`,
            }}
          />
        )}
        {[...panel.tabs]
          .sort((a, b) => (a === "git" ? -1 : b === "git" ? 1 : 0))
          .filter((k) => k !== "file" || !separate || files === null || sessionId === null)
          .map((kind) => (
            <span
              data-item-id={kind}
              key={kind}
              className="ptab-group"
              data-selected={kind === active}
            >
              <Tab
                data-testid="shell.side-panel-open-tab"
                type="button"

                aria-selected={kind === active}
                data-tab={kind}
                title={`Show ${TAB_META[kind].label} tab${kind === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
                aria-label={`Show ${TAB_META[kind].label} tab${kind === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
                onClick={() => {
                  shell.actions.openTab(kind);
                }}
              >
                <Icon name={TAB_ICON[kind]} size={14} />
                <span className="ptab-label">{TAB_META[kind].label}</span>
              </Tab>
              <TabClose
                data-testid="shell.side-panel-close-tab"
                type="button"

                aria-label={`Close ${TAB_META[kind].label} tab`}
                title={`Close ${TAB_META[kind].label} tab`}
                onClick={() => {
                  shell.actions.closeTab(kind);
                }}
              >
                <Icon name="x" size={12} />
              </TabClose>
            </span>
          ))}
        {files !== null && sessionId !== null && panel.tabs.includes("file") && (
          <FilePanelTabs files={files} sessionId={sessionId} active={active === "file"} />
        )}
        <span className="end">
          {remaining !== undefined && (
            <IconButton
              data-testid="shell.side-panel-open-tab-2"
              icon="plus"
              label={`Open ${TAB_META[remaining].label} tab${remaining === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
              title={`Open ${TAB_META[remaining].label} tab${remaining === "terminal" ? ` (${formatKeys("Mod+`")})` : ""}`}
              onClick={() => {
                shell.actions.openTab(remaining);
              }}
            />
          )}
          <IconButton
            data-testid="shell.side-panel-maximize-2"
            icon="maximize-2"
            label={
              layout.editorSplit
                ? layout.slot === "folded"
                  ? "Show chat"
                  : "Full width"
                : layout.maximized
                  ? "Restore side panel"
                  : "Maximize side panel"
            }
            title={
              layout.editorSplit
                ? layout.slot === "folded"
                  ? "Show chat"
                  : "Full width"
                : layout.maximized
                  ? "Restore side panel"
                  : "Maximize side panel"
            }
            aria-pressed={layout.maximized}
            onClick={() => {
              if (layout.editorSplit) {
                if (layout.slot === "folded") shell.actions.unfoldChat();
                else shell.actions.foldChat();
              } else shell.actions.toggleMaximized();
            }}
          />
          <IconButton
            data-testid="shell.side-panel-x"
            icon="x"
            label="Close the panel"
            title="Close the panel"
            onClick={shell.actions.closePanel}
          />
        </span>
      </Tabs>
      <div data-testid="shell.side-panel-pbody" className="pbody" role="tabpanel">
        {body ?? <div className="empty">Nothing to show.</div>}
      </div>
    </section>
  );
}
