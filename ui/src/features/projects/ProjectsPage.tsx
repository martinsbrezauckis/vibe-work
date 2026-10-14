/**
 * The Projects overview (from the left rail's Projects button):
 * every project Vibe Work knows as a card, built from each handbook's manifest head and active plan heads without
 * opening a session; a "Needs you" strip across projects; a filter by project status and a search; the selected
 * project's handbook in a detail pane. Defaults (the user may change them): Work stays the start view, cards sort by
 * most recent activity, up to two active plans per card.
 * Depends on: state/projects (controller), model.ts, ProjectCardView, ProjectDetail, shell hooks, components/Icon,
 * components/Selector.
 * Called by: shell/Content (when the page is open).
 * the cards are the user's projects (`overviewItems`), selection by project id; "New project" opens the
 * list's name field.
 * Invariants: renders only (every decision is in model.ts or the controller); page text is plain text; nothing here
 * writes a file.
 */
import { type ReactElement, useEffect, useMemo, useState } from "react";
import { EmptyState } from "../../components/EmptyState";

import { Icon } from "../../components/Icon";
import { Selector } from "../../components/Selector";
import { useStore } from "../../state/createStore";
import type { Projects, StatusFilter } from "../../state/projects";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { useSessionSummaries, useShellState } from "../shell/hooks";
import { useGroups } from "../sessions/useGroups";
import {
  STATUS_LABEL,
  STATUS_ORDER,
  ago,
  filterCounts,
  groupItems,
  healthOf,
  needsYou,
  overviewItems,
  sessionsOf,
} from "./model";
import { NoFolderCard, ProjectCardView } from "./ProjectCardView";
import { ProjectDetail } from "./ProjectDetail";
import { ProjectSettings } from "./ProjectSettings";

const FILTERS: ReadonlyArray<[StatusFilter, string]> = [
  ["all", "All"],
  ...STATUS_ORDER.map((s): [StatusFilter, string] => [s, STATUS_LABEL[s]]),
];

function Page({ projects }: { projects: Projects }): ReactElement {
  const keyBlocked = useKeyReadiness() !== null;
  const { sessions: sessionActions, composer } = useApp();
  useStore(composer.store, (s) => s.sending);
  useShellState((s) => s.selected);
  const state = useStore(projects.store, (s) => s);
  const sessions = useSessionSummaries();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, 60_000);
    return () => {
      clearInterval(timer);
    };
  }, []);
  const data = useGroups((g) => g.data);
  const members = data?.members ?? null;
  const cards = useMemo(() => state.cards ?? [], [state.cards]);
  const items = useMemo(() => overviewItems(cards, data), [cards, data]);
  const groups = useMemo(
    () => groupItems(items, sessions, members, state.filter, state.query),
    [items, sessions, members, state.filter, state.query],
  );
  const openSession = (id: string) => {
    projects.openSession(id);
  };
  const counts = filterCounts(items);
  const needs = needsYou(sessions, items, members);
  const running = sessions.filter((s) => s.badge === "run" && !s.archived).length;
  const selected = items.find((i) => i.project.id === state.selected) ?? null;
  return (
    <div
      className="pj"
      data-detail={selected !== null}
      data-settings={state.settings !== null}
      onKeyDown={(e) => {
        if (e.key === "Escape" && selected !== null) {
          if (state.settings !== null) projects.closeSettings();
          else projects.select(null);
        }
      }}
    >
      <section className="pj-overview" aria-labelledby="pj-title">
        <div className="pj-head">
          <div>
            <h2 id="pj-title">Projects</h2>
            <div className="pj-sub">
              {counts.all} project{counts.all === 1 ? "" : "s"} · {running} session
              {running === 1 ? "" : "s"} running
            </div>
          </div>
          <div className="pj-tools">
            <label className="pj-filter">
              <Icon name="search" size={16} />
              <input
                value={state.query}
                placeholder="Filter projects"
                aria-label="Filter projects"
                onChange={(e) => {
                  projects.setQuery(e.target.value);
                }}
              />
            </label>
            <Selector
              label="Project status"
              value={state.filter}
              options={FILTERS.map(([key, label]) => ({
                value: key,
                label: `${label} ${String(counts[key])}`,
              }))}
              onChange={(key) => {
                projects.setFilter(key);
              }}
            />
            <button
              type="button"
              className="pj-btn"
              title="Choose a folder and start a session in it; it then shows here"
              onClick={() => {
                void sessionActions?.pickFolder().then((folder) => {
                  if (folder !== null) projects.close();
                });
              }}
            >
              <Icon name="folder-plus" size={16} />
              Add project folder
            </button>
            <button
              type="button"
              className="pj-btn"
              title="A project of your own: a name now, a main folder later if you like"
              onClick={() => {
                projects.close();
                projects.groups.setCreating(true);
              }}
            >
              <Icon name="plus" size={16} />
              New project
            </button>
            <button
              type="button"
              className="pj-btn"
              aria-label="Read the projects again"
              title="Read the projects again"
              disabled={state.loading}
              onClick={() => void projects.refresh()}
            >
              <Icon name="refresh-cw" size={16} />
            </button>
          </div>
        </div>
        <KeyNotice message="Handbook requests use your Mistral key so the agent can write pages and plans." />
        {state.error !== null && (
          <p className="pj-error" role="alert">
            {state.error}
          </p>
        )}
        {needs.length > 0 && (
          <section className="pj-needs" aria-labelledby="pj-needs-h">
            <h3 id="pj-needs-h">
              Needs you <span className="count">{needs.length}</span>
            </h3>
            <ul>
              {needs.map((n) => (
                <li key={n.sessionId}>
                  <button
                    type="button"
                    onClick={() => {
                      projects.openSession(n.sessionId);
                    }}
                  >
                    <span className="kind">Needs input</span>
                    <span>
                      <span className="what">{n.title}</span>{" "}
                      <span className="where">in {n.where}</span>
                    </span>
                    <span className="age">{ago(n.bumpedAt, now)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
        {state.cards === null ? (
          // skeletons shaped like the cards (a slow shimmer) instead of a line of text.
          <div className="pj-grid pj-skeleton" role="status" aria-label="Reading the projects…">
            {[0, 1, 2].map((i) => (
              <div key={i} className="pj-card sk-card" aria-hidden="true">
                <span className="sk" style={{ width: "55%" }} />
                <span className="sk" style={{ width: "35%" }} />
                <span className="sk sk-bar" />
                <span className="sk" style={{ width: "70%" }} />
              </div>
            ))}
            <span className="sr">Reading the projects…</span>
          </div>
        ) : groups.length === 0 ? (
          <EmptyState glyph="folder" className="empty">
            {items.length === 0
              ? "No projects yet: start a session in a folder, or create one with New project."
              : `No project matches “${state.query}”.`}
          </EmptyState>
        ) : (
          groups.map((g) => (
            <section key={g.status} className="pj-group" data-status={g.status}>
              <h3>
                {STATUS_LABEL[g.status]} <span>{g.items.length}</span>
              </h3>
              <div className="pj-grid">
                {g.items.map((item) => {
                  const id = item.project.id;
                  const select = () => {
                    projects.select(state.selected === id ? null : id);
                  };
                  const chips = sessionsOf(item, sessions, members);
                  return item.card === null ? (
                    <NoFolderCard
                      key={id}
                      title={item.project.name}
                      projectId={id}
                      sessions={chips}
                      selected={state.selected === id}
                      select={select}
                      chooseFolder={() => void projects.chooseFolder(id)}
                      openSession={openSession}
                    />
                  ) : (
                    <ProjectCardView
                      key={id}
                      card={item.card}
                      title={item.project.name}
                      projectId={id}
                      sessions={chips}
                      health={healthOf(item.card, state.hubs[item.card.project])}
                      selected={state.selected === id}
                      now={now}
                      select={select}
                      start={(trustedClick) => void projects.startHandbook(id, trustedClick)}
                      requestBusy={keyBlocked || projects.handbookRequestBusy(id)}
                      openSession={openSession}
                    />
                  );
                })}
              </div>
            </section>
          ))
        )}
      </section>
      {selected !== null && state.settings !== null && (
        <ProjectSettings
          key={selected.project.id}
          projectId={selected.project.id}
          title={selected.project.name}
          folder={selected.project.mainFolder}
          close={() => {
            projects.closeSettings();
          }}
          chooseFolder={() => void projects.chooseFolder(selected.project.id)}
        />
      )}
      {selected !== null && state.settings === null && (
        <ProjectDetail
          key={selected.project.id}
          projectId={selected.project.id}
          title={selected.project.name}
          card={selected.card}
          hub={selected.card === null ? null : state.hubs[selected.card.project]}
          sessions={sessionsOf(selected, sessions, members)}
          requestBusy={keyBlocked || projects.handbookRequestBusy(selected.project.id)}
          actions={{
            close: () => {
              projects.select(null);
            },
            openHandbook: (path) => void projects.openHandbook(selected.project.id, path),
            newSession: () => void projects.newSession(selected.project.id),
            reveal: () => void projects.reveal(selected.project.id),
            start: (trustedClick) => void projects.startHandbook(selected.project.id, trustedClick),
            askPlan: (trustedClick) => void projects.writePlan(selected.project.id, trustedClick),
            migrate: () => void projects.migrate(selected.project.id),
            openSession,
            chooseFolder: () => void projects.chooseFolder(selected.project.id),
          }}
        />
      )}
    </div>
  );
}

/** The page; nothing without a core. */
export function ProjectsPage(): ReactElement | null {
  const { projects } = useApp();
  if (projects === null) return null;
  return <Page projects={projects} />;
}
