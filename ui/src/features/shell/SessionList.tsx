/**
 * The list column (Work view): New chat (the same as the rail's New chat and Ctrl N), a filter with the archive toggle, "Needs you", the
 * pinned shelf, the user's projects with their sessions and "Not in a project". Everything is derived
 * from the core model and the project groups by `buildList`; the rows, menus and dialogs live in
 * `features/sessions/`.
 * Depends on: AppContext, hooks, sessions/* (incl. the move menu and new-project field),
 * components/Icon. Called by: Shell.
 * Invariants: one list for every window size (the narrow-window drawer is this same column);
 * the filter searches title, preview, folder and model across all loaded pages and asks Vibe for
 * older pages on demand (`useSessionWiring`); archived sessions show only while the archive toggle is on.
 */
import { useMemo, useRef } from "react";
import { EmptyState } from "../../components/EmptyState";
import { IconButton } from "../../components/Controls";

import { Icon } from "../../components/Icon";
import { useModalFocus } from "../../hooks/useModalFocus";
import { formatKeys } from "../../state/shortcuts";
import { summarize } from "../../state/sessionSummary";
import { Badge } from "../sessions/Badge";
import { buildList } from "../sessions/listModel";
import { useListDrop } from "../sessions/listDrop";
import { useListMotion } from "../sessions/listMotion";
import { MoveMenu } from "../sessions/MoveMenu";
import { NewProjectField } from "../sessions/NewProjectField";
import { ProjectGroupView } from "../sessions/ProjectGroupView";
import { SessionRows } from "../sessions/SessionRows";
import { SettingsNav } from "../settings/SettingsNav";
import { UnfiledSection } from "../sessions/UnfiledSection";
import { useGroupActions, useGroups } from "../sessions/useGroups";
import { useApp } from "./AppContext";
import { useCoreState, useLayout, useShellState } from "./hooks";
import { openNewChat } from "./newChat";
import { CopiesButton } from "../worktrees/CopiesButton";

/** The list column. */
export function SessionList() {
  const app = useApp();
  const { shell, core, sessions } = app;
  const filter = useShellState((s) => s.filter);
  const entries = useCoreState((s) => s.sessions);
  const index = useCoreState((s) => s.index);
  const stuck = useCoreState((s) => s.stuck);
  const layout = useLayout();
  const groups = useGroupActions();
  const data = useGroups((g) => g.data);
  const creating = useGroups((g) => g.creating);
  // one drop controller for the whole list (sessions into projects, projects reordered).
  const drop = useListDrop(groups, () => groups?.store.get().data?.projects.map((p) => p.id) ?? []);
  const summaries = useMemo(
    () => Object.values(entries).map((entry) => summarize(entry, stuck)),
    [entries, stuck],
  );
  const full = useMemo(
    () => buildList(summaries, { query: "", showArchived: index.includeArchived }, data),
    [summaries, index.includeArchived, data],
  );
  const model =
    filter.trim() === ""
      ? full
      : buildList(
          summaries,
          {
            query: filter,
            showArchived: index.includeArchived,
          },
          data,
        );
  const searching = filter.trim() !== "";
  // Narrow windows: the list is an overlay drawer (modal while open: focus moves in, Esc and a
  // click outside close it, focus returns to the button that opened it).
  const drawerOpen = layout.listDrawerOpen;
  // The Settings page shows its categories in this column.
  const settings = useShellState((s) => s.settings.open);
  const root = useRef<HTMLElement>(null);
  const closeDrawer = () => {
    shell.actions.setDrawer(false);
  };
  useModalFocus(drawerOpen, root, closeDrawer);
  const scroller = useRef<HTMLDivElement>(null);
  useListMotion(scroller, filter);
  const shownGroups = new Map(model.projects.map((group) => [group.key, group]));
  return (
    <aside
      className={`column${drawerOpen ? " drawer-open" : ""}${settings ? " is-settings" : ""}`}
      id="column"
      aria-label={settings ? "Settings" : "Sessions"}
      ref={root}
      tabIndex={drawerOpen ? -1 : undefined}
      {...(drawerOpen ? { role: "dialog", "aria-modal": true } : {})}
    >
      <div className="drawer-scrim" onClick={closeDrawer} />
      {settings && (
        <div className="column-inner">
          <SettingsNav />
        </div>
      )}
      {/* While Settings is open the session list stays mounted underneath (its scroll and folds kept), inert. */}
      <div className={`column-inner${settings ? " behind" : ""}`} inert={settings}>
        <div className="col-top">
          <button
            type="button"
            className="btn-primary"
            title={`New chat (${formatKeys("Mod+N")})`}
            onClick={() => {
              openNewChat(app);
            }}
          >
            <Icon name="message-square-plus" />
            New chat
          </button>
          <div className="filter-row">
            <label className="filter">
              <Icon name="search" />
              <input
                aria-label="Filter sessions"
                placeholder="Filter sessions"
                value={filter}
                onChange={(e) => {
                  shell.actions.setFilter(e.target.value);
                }}
              />
            </label>
            <IconButton
              icon="archive"
              aria-label="Show archived sessions"
              aria-pressed={index.includeArchived}
              title={index.includeArchived ? "Hide archived sessions" : "Show archived sessions"}
              onClick={() => void core?.setIncludeArchived(!index.includeArchived)}
            />
          </div>
        </div>
        <div className="col-scroll" ref={scroller} {...drop}>
          {full.needs.length > 0 && (
            <div hidden={model.needs.length === 0}>
              <div className="sec">
                <span>Needs you</span>
                <span className="mono">{model.needs.length}</span>
              </div>
              {full.needs.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  className="needs-row"
                  data-session-id={s.id}
                  onClick={() => void sessions?.open(s.id)}
                >
                  <Badge kind="input" />
                  <span className="t">{s.title}</span>
                  <span className="p">{s.project}</span>
                </button>
              ))}
            </div>
          )}
          {full.pinned.length > 0 && (
            <div hidden={model.pinned.length === 0}>
              <div className="sec">
                <span>Pinned</span>
                <span className="mono">{model.pinned.length}</span>
              </div>
              <div className="pinned">
                <SessionRows rows={full.pinned} shown={model.pinned} />
              </div>
            </div>
          )}
          <div className="sec">
            <span>Projects</span>
            {groups !== null && model.grouped && (
              <button
                type="button"
                className="sec-btn"
                aria-label="New project"
                title="New project: a name now, a main folder later if you like"
                onClick={() => {
                  groups.setCreating(true);
                }}
              >
                <Icon name="plus" size={14} />
              </button>
            )}
          </div>
          {creating && <NewProjectField />}
          {full.projects.map((group) => (
            <ProjectGroupView
              key={group.key}
              group={group}
              shown={shownGroups.get(group.key) ?? null}
            />
          ))}
          {full.grouped && <UnfiledSection rows={full.unfiled} shown={model.unfiled} />}
          {model.total === 0 && index.loaded && (
            <EmptyState glyph="chat" small className="col-note">
              {searching ? "No sessions match." : "No sessions yet."}
            </EmptyState>
          )}
          {index.error !== null && (
            <p className="col-note error">Could not load sessions: {index.error}</p>
          )}
          {model.hiddenArchived > 0 && !searching && (
            <p className="col-note">{model.hiddenArchived} archived hidden.</p>
          )}
          {index.nextCursor !== null && (
            <button
              type="button"
              className="more-btn"
              disabled={index.loading}
              onClick={() => void core?.loadMore()}
            >
              {index.loading ? "Loading..." : searching ? "Search older sessions" : "Load more"}
            </button>
          )}
        </div>
        <MoveMenu />
        <div className="col-foot">
          <CopiesButton />
          {layout.phone && (
            <button
              type="button"
              className="rbtn-wide"
              onClick={() => {
                shell.actions.openSettings();
              }}
            >
              <Icon name="settings" />
              Settings
            </button>
          )}
        </div>
      </div>
    </aside>
  );
}
