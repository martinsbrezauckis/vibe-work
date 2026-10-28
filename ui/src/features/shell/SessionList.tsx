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
import { TEXT } from "../../vocabulary";
import { useMemo, useRef } from "react";
import { EmptyState } from "../../components/EmptyState";
import { Badge, Button, IconButton } from "../../components/Controls";

import { Icon } from "../../components/Icon";
import { useModalFocus } from "../../hooks/useModalFocus";
import { formatKeys } from "../../state/shortcuts";
import { summarize } from "../../state/sessionSummary";
import { ActivityMark } from "../sessions/ActivityMark";
import { buildList } from "../sessions/listModel";
import { useListDrop } from "../sessions/listDrop";
import { useListMotion } from "../sessions/listMotion";
import { MoveMenu } from "../sessions/MoveMenu";
import { NewProjectField } from "../sessions/NewProjectField";
import { ProjectGroupView } from "../sessions/ProjectGroupView";
import { SessionRows } from "../sessions/SessionRows";
import { SessionListSkeleton } from "../sessions/SessionListSkeleton";
import { SettingsNav } from "../settings/SettingsNav";
import { UnfiledSection } from "../sessions/UnfiledSection";
import { useGroupActions, useGroups } from "../sessions/useGroups";
import { useApp } from "./AppContext";
import { useCoreState, useLayout, useShellState } from "./hooks";
import { openNewChat } from "./newChat";
import { badgeText, badgeTitle } from "../sessions/newChatText";
import { NO_NEW_CHAT } from "../../state/newChatDraft";
import { useStore } from "../../state/createStore";
import { CopiesButton } from "../worktrees/CopiesButton";

/** The list column. */
export function SessionList() {
  const app = useApp();
  // While the user is in a session and the New chat screen holds a terminal or a page, its button says so.
  const draftOpen = useStore(app.newChat?.store ?? NO_NEW_CHAT, (s) => s.open);
  const away = useShellState((s) => s.selected !== null);
  const badge = away ? badgeText(draftOpen) : null;
  const { shell, core, sessions } = app;
  const filter = useShellState((s) => s.filter);
  const entries = useCoreState((s) => s.sessions);
  const index = useCoreState((s) => s.index);
  const ready = useCoreState((s) => s.server.phase === "running");
  const stuck = useCoreState((s) => s.stuck);
  const layout = useLayout();
  const groups = useGroupActions();
  const data = useGroups((g) => g.data);
  const groupError = useGroups((g) => g.error);
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
  const error = index.loading ? null : index.error;
  const loading =
    error === null &&
    (!index.loaded ||
      (groups !== null && data === null && groupError === null) ||
      (index.loading && full.total === 0));
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
      data-testid="shell.session-list-aside-surface-1"
      className={`column${drawerOpen ? " drawer-open" : ""}${settings ? " is-settings" : ""}`}
      id="column"
      aria-label={settings ? "Settings" : "Sessions"}
      ref={root}
      tabIndex={drawerOpen ? -1 : undefined}
      {...(drawerOpen ? { role: "dialog", "aria-modal": true } : {})}
    >
      <div
        data-testid="shell.session-list-drawer-scrim"
        className="drawer-scrim"
        onClick={closeDrawer}
      />
      {settings && (
        <div className="column-inner">
          <SettingsNav />
        </div>
      )}
      {/* While Settings is open the session list stays mounted underneath (its scroll and folds kept), inert. */}
      <div className={`column-inner${settings ? " behind" : ""}`} inert={settings}>
        <div className="col-top">
          <div className="session-list-header">
            <IconButton
              data-testid="shell.fold-sessions"
              icon="panel-left"
              label="Fold sessions"
              onClick={() => {
                if (layout.editorSplit) shell.actions.unfoldChat();
                else shell.actions.toggleList(true);
              }}
            />
            <Button
              data-testid="shell.session-list-open-new-chat"
              type="button"
              appearance="dialog"
              title={
                badge === null
                  ? `${TEXT.newChat} (${formatKeys("Mod+N")})`
                  : badgeTitle(draftOpen, formatKeys("Mod+N"))
              }
              onClick={() => {
                openNewChat(app);
              }}
            >
              <Icon name="message-square-plus" />
              {TEXT.newChat}
              {badge !== null && <Badge>{badge}</Badge>}
            </Button>
          </div>
          <div className="filter-row">
            <label className="filter">
              <Icon name="search" />
              <input
                data-testid="shell.session-list-set-filter"
                aria-label="Filter sessions"
                placeholder="Filter sessions"
                value={filter}
                onChange={(e) => {
                  shell.actions.setFilter(e.target.value);
                }}
              />
            </label>
            <IconButton
              data-testid="shell.session-list-set-include-archived"
              icon="archive"
              label="Show archived sessions"
              aria-pressed={index.includeArchived}
              title={index.includeArchived ? "Hide archived sessions" : "Show archived sessions"}
              onClick={() => void core?.setIncludeArchived(!index.includeArchived)}
            />
          </div>
        </div>
        <div className="col-scroll" ref={scroller} {...drop}>
          <div className="pj-skeleton" hidden={!loading}>
            {loading && <SessionListSkeleton ready={ready} />}
          </div>
          <div hidden={loading}>
            {full.needs.length > 0 && (
              <div hidden={model.needs.length === 0}>
                <div className="sec">
                  <span>Needs you</span>
                  <span className="mono">{model.needs.length}</span>
                </div>
                {full.needs.map((s) => (
                  <button
                    data-testid="shell.session-list-open"
                    data-item-id={s.id}
                    key={s.id}
                    type="button"
                    className="needs-row"
                    data-session-id={s.id}
                    onClick={() => void sessions?.open(s.id)}
                  >
                    <ActivityMark kind="input" />
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
                  data-testid="shell.session-list-set-creating"
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
            {model.total === 0 && index.loaded && error === null && (
              <EmptyState glyph="chat" small className="col-note">
                {searching ? "No sessions match." : "No sessions yet."}
              </EmptyState>
            )}
            {error !== null && (
              <div className="col-note error">
                <p>Could not load your sessions.</p>
                <p className="ui-help">{error}</p>
                <Button
                  data-testid="sessions.session-list-retry"
                  type="button"

                  shape="outlined"
                  disabled={index.loading}
                  onClick={() => {
                    void core?.refresh();
                    void groups?.refresh();
                  }}
                >
                  Try again
                </Button>
              </div>
            )}
            {model.hiddenArchived > 0 && !searching && (
              <p className="col-note">{model.hiddenArchived} archived hidden.</p>
            )}
            {index.nextCursor !== null && (
              <button
                data-testid="shell.session-list-load-more"
                type="button"
                className="more-btn"
                disabled={index.loading}
                onClick={() => void core?.loadMore()}
              >
                {index.loading ? "Loading…" : searching ? "Search older sessions" : "Load more"}
              </button>
            )}
          </div>
        </div>
        <MoveMenu />
        <div className="col-foot">
          <CopiesButton />
          {layout.phone && (
            <button
              data-testid="shell.session-list-open-settings"
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
