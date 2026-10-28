/**
 * The Projects overview's detail pane (the selected project's handbook): the project's handbook without opening a session: name, summary, actions (open the handbook,
 * new session, show the folder, start a handbook), the active plans with every step, the sessions, the
 * handbook's findings and its pages by kind. Opening a page goes to a session's Project tab.
 * Depends on: model.ts, useProjectActions, ProjectCardView (chips), pages/HubRows, Icon, protocol types.
 * Called by: ProjectsPage.
 * The pane belongs to one of the user's projects (`title` is its name); without a main folder (`card`
 * null) it shows the project's sessions and "Choose main folder…".
 * Invariants: renders only; page text is plain text; handbook requests send user messages;
 * the agent writes the files, the app never does.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useEffect, useRef } from "react";

import { Icon } from "../../components/Icon";
import type { ProjectCard, ProjectHub } from "../../protocol/core";
import { Findings, Groups } from "../pages/HubRows";
import { HANDBOOK_BUSY_TOOLTIP } from "../pages/handbookPrompts";
import { type CardSession, displayName, planLine } from "./model";
import { SessionChip } from "./ProjectCardView";
import { useProjectActions } from "./useProjectActions";
import { useApp } from "../shell/AppContext";

function SettingsButton({ projectId }: { projectId: string }) {
  const { projects } = useApp();
  return (
    <Button
      data-testid="projects.project-detail-open-settings"
      type="button"
      appearance="project"
      shape="outlined"
      onClick={() => {
        projects?.openSettings(projectId);
      }}
    >
      Project settings
    </Button>
  );
}

/** What the pane's buttons do. */
export interface DetailActions {
  close: () => void;
  openHandbook: (path: string | null) => void;
  /** Opens Files without starting a session. */
  openFiles?: () => void;
  /** Opens a terminal in the project's folder without starting a session. */
  openTerminal?: () => void;
  newSession: () => void;
  reveal: () => void;
  start: (trustedClick: boolean) => void;
  /** Sends the first plan request in the project's chat. */
  askPlan: (trustedClick: boolean) => void;
  openSession: (id: string) => void;
  /** "Choose main folder…". */
  chooseFolder: () => void;
}

function Steps({ plan }: { plan: ProjectCard["plans"][number] }): ReactElement {
  const line = planLine(plan);
  return (
    <div className="pj-dsec">
      <h4>
        <span>{plan.title}</span>
        <span>
          {line.done} of {line.total}
        </span>
      </h4>
      <ul className="pj-steps">
        {plan.steps.map((s) => (
          <li key={s.id} data-status={s.status}>
            <span className="m" aria-hidden="true" />
            <span>
              {s.text}
              {s.status === "skipped" && " (skipped)"}
              {s.status === "blocked" && " (blocked)"}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The pane of a project without a main folder: its sessions and "Choose main folder…". */
function NoFolderDetail({
  projectId,
  title,
  sessions,
  actions,
}: {
  projectId: string;
  title: string;
  sessions: readonly CardSession[];
  actions: DetailActions;
}): ReactElement {
  const management = useProjectActions(projectId, title);
  const closeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    closeRef.current?.focus();
  }, [projectId]);
  return (
    <aside
      data-testid="projects.project-detail-pj-detail"
      className="pj-detail"
      aria-label="Project details"
      onContextMenu={management.onContextMenu}
    >
      <div className="pj-dh">
        <div className="row">
          <Icon name="folder" />
          {management.renameField ?? <h3>{title}</h3>}
          {management.button}
          <button
            data-testid="projects.project-detail-pj-x"
            ref={closeRef}
            type="button"
            className="pj-x"
            aria-label="Close details"
            onClick={actions.close}
          >
            <Icon name="x" />
          </button>
        </div>
        <p>This project has no main folder, so there is no handbook to show.</p>
        <div className="pj-acts">
          <SettingsButton projectId={projectId} />
          <Button
            data-testid="projects.project-detail-pj-btn"
            type="button"
            appearance="project"
            shape="outlined"
            variant="primary"
            onClick={actions.chooseFolder}
          >
            Choose main folder…
          </Button>
          <Button
            data-testid="projects.project-detail-pj-btn-2"
            type="button"
            appearance="project"
            shape="outlined"
            onClick={actions.newSession}
          >
            New session
          </Button>
        </div>
      </div>
      {sessions.length > 0 && (
        <div className="pj-dsec">
          <h4>
            <span>Sessions</span>
            <span>{sessions.length}</span>
          </h4>
          <div className="pj-sess">
            {sessions.map((s) => (
              <SessionChip key={s.id} session={s} open={actions.openSession} />
            ))}
          </div>
        </div>
      )}
      {management.menu}
    </aside>
  );
}

/** The pane for a project: `card` is its main folder's handbook card (`null`: no main folder). */
export function ProjectDetail({
  projectId,
  title,
  card,
  hub,
  sessions,
  actions,
  requestBusy = false,
}: {
  projectId: string;
  title: string;
  card: ProjectCard | null;
  hub: ProjectHub | null | undefined;
  sessions: readonly CardSession[];
  actions: DetailActions;
  requestBusy?: boolean;
}): ReactElement {
  if (card === null)
    return (
      <NoFolderDetail projectId={projectId} title={title} sessions={sessions} actions={actions} />
    );
  return (
    <FolderDetail
      projectId={projectId}
      title={title}
      card={card}
      hub={hub}
      sessions={sessions}
      actions={actions}
      requestBusy={requestBusy}
    />
  );
}

function FolderDetail({
  projectId,
  title,
  card,
  hub,
  sessions,
  actions,
  requestBusy,
}: {
  projectId: string;
  title: string;
  card: ProjectCard;
  hub: ProjectHub | null | undefined;
  sessions: readonly CardSession[];
  actions: DetailActions;
  requestBusy: boolean;
}): ReactElement {
  const management = useProjectActions(projectId, title);
  const closeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    closeRef.current?.focus();
  }, [projectId]);
  const book = hub?.hub ?? null;
  const findings = (book?.validation.findings ?? []).filter((f) => f.severity !== "info");
  const summary = card.manifest?.description ?? null;
  return (
    <aside
      data-testid="projects.project-detail-pj-detail-2"
      className="pj-detail"
      aria-label="Project details"
      onContextMenu={management.onContextMenu}
    >
      <div className="pj-dh">
        <div className="row">
          <Icon name="book-open" />
          {management.renameField ?? <h3>{title}</h3>}
          {management.button}
          <button
            data-testid="projects.project-detail-pj-x-2"
            ref={closeRef}
            type="button"
            className="pj-x"
            aria-label="Close details"
            onClick={actions.close}
          >
            <Icon name="x" />
          </button>
        </div>
        <p>
          {displayName(card) === title ? "" : `${displayName(card)} · `}
          {summary ?? card.project}
        </p>
        <div className="pj-acts">
          <SettingsButton projectId={projectId} />
          {actions.openFiles && (
            <Button
              data-testid="projects.open-files"
              appearance="project"
              shape="outlined"
              onClick={actions.openFiles}
            >
              Open Files
            </Button>
          )}
          {actions.openTerminal && (
            <Button
              data-testid="projects.open-terminal"
              appearance="project"
              shape="outlined"
              onClick={actions.openTerminal}
            >
              Open terminal
            </Button>
          )}
          {card.state === "none" ? (
            <Button
              data-testid="projects.project-detail-start"
              type="button"
              appearance="project"
              shape="outlined"
              variant="primary"
              onClick={(event) => {
                actions.start(event.nativeEvent.isTrusted);
              }}
              disabled={requestBusy}
              title={requestBusy ? HANDBOOK_BUSY_TOOLTIP : undefined}
            >
              Ask Vibe to create the handbook
            </Button>
          ) : (
            <Button
              data-testid="projects.project-detail-open-handbook"
              type="button"
              appearance="project"
              shape="outlined"
              variant="primary"
              title="Opens Work in this folder with the Project tab on the handbook"
              onClick={() => {
                actions.openHandbook(null);
              }}
            >
              {sessions.length > 0 ? "Open handbook" : "Open in a new session"}
            </Button>
          )}
          <Button
            data-testid="projects.project-detail-pj-btn-4"
            type="button"
            appearance="project"
            shape="outlined"
            onClick={actions.newSession}
          >
            New session
          </Button>
          <Button
            data-testid="projects.project-detail-pj-btn-5"
            type="button"
            appearance="project"
            shape="outlined"
            onClick={actions.reveal}
          >
            <Icon name="folder" size={14} />
            Show folder
          </Button>
        </div>
      </div>
      {card.state === "none" && (
        <div className="pj-dsec">
          <div className="pj-nohb">
            This project has no handbook yet to keep its plans, decisions and reports together.
          </div>
        </div>
      )}
      {card.plans.map((plan) => (
        <Steps key={plan.path} plan={plan} />
      ))}
      {book !== null &&
        !book.truncated &&
        !book.groups.some((g) => g.pages.some((p) => p.kind === "plan")) && (
          <div className="pj-dsec">
            <button
              data-testid="projects.project-detail-ask-plan"
              type="button"
              className="pages-plan-link"
              onClick={(event) => {
                actions.askPlan(event.nativeEvent.isTrusted);
              }}
              disabled={requestBusy}
              title={requestBusy ? HANDBOOK_BUSY_TOOLTIP : undefined}
            >
              Ask Vibe to write a plan page
            </button>
          </div>
        )}
      {sessions.length > 0 && (
        <div className="pj-dsec">
          <h4>
            <span>Sessions</span>
            <span>{sessions.length}</span>
          </h4>
          <div className="pj-sess">
            {sessions.map((s) => (
              <SessionChip key={s.id} session={s} open={actions.openSession} />
            ))}
          </div>
        </div>
      )}
      {findings.length > 0 && (
        <div className="pj-dsec">
          <h4>
            <span>Handbook health</span>
            <span>{findings.length}</span>
          </h4>
          <Findings findings={findings.slice(0, 12)} label="Handbook findings" />
        </div>
      )}
      {card.state !== "none" && hub === null && <p className="pj-note">Reading the handbook…</p>}
      {book !== null && (
        <div className="pj-dsec pj-pages">
          <Groups
            groups={book.groups}
            counts={{}}
            findingsOf={() => []}
            open={(path) => {
              actions.openHandbook(path);
            }}
          />
        </div>
      )}
      {management.menu}
    </aside>
  );
}
