/**
 * One project card of the Projects overview (hierarchy: name > current plans >
 * sessions > handbook health > path): the manifest's name and summary, up to two active plans with their step
 * progress and current step, the project's sessions as chips, and a footer with the folder, the last activity and the
 * health line. A folder without a handbook offers a creation request instead of plans.
 * Depends on: model.ts, useProjectActions, components/Icon, protocol types. Called by: ProjectsPage.
 * The card belongs to one of the user's projects (its name is the project's; the handbook's own name,
 * when different, leads the summary); a project without a main folder gets `NoFolderCard` (its sessions and "Choose
 * main folder…").
 * Invariants: renders only; the card selects on click and through its name button (keyboard); inner buttons do not
 * also select the card; all page text is plain text.
 */
import { Card } from "../../components/Card";
import { Badge, Button } from "../../components/Controls";
import type { MouseEvent, ReactElement } from "react";

import { Icon } from "../../components/Icon";
import { HANDBOOK_BUSY_TOOLTIP } from "../pages/handbookPrompts";
import { PathText } from "../../components/PathText";
import type { ProjectCard } from "../../protocol/core";
import { type CardSession, type Health, type PlanLine, ago, displayName, planLine } from "./model";
import { useProjectActions } from "./useProjectActions";

/** A plan's line, bar and current step. */
function PlanBar({ line }: { line: PlanLine }): ReactElement {
  return (
    <div className="pj-plan">
      <div className="pj-pl">
        <b>{line.title}</b>
        <span>
          {line.done} of {line.total}
        </span>
      </div>
      <div
        className="pj-bar"
        role="img"
        aria-label={`${String(line.done)} of ${String(line.total)} steps done`}
      >
        <i style={{ width: `${String(line.donePct)}%` }} />
        <i className="now" style={{ width: `${String(line.nowPct)}%` }} />
      </div>
      {line.now !== null && (
        <div className="pj-nowstep">
          Now: <em>{line.now}</em>
        </div>
      )}
    </div>
  );
}

/** A session chip; a button that opens the session. */
export function SessionChip({
  session,
  open,
}: {
  session: CardSession;
  open: (id: string) => void;
}): ReactElement {
  const note =
    session.state === "waiting" ? " · needs you" : session.state === "idle" ? " · idle" : "";
  return (
    <Badge
      data-item-id={session.id}
      data-testid="projects.project-card-view-stop-propagation"
      type="button"
      presentation="project-session"
      as="button"
      data-state={session.state}
      title="Open this session"
      onClick={(e: MouseEvent) => {
        e.stopPropagation();
        open(session.id);
      }}
    >
      <span className="d" aria-hidden="true" />
      {session.title}
      {note}
    </Badge>
  );
}

/** One card. `title` is the project's name, `projectId` its id. */
export function ProjectCardView({
  card,
  title,
  projectId,
  sessions,
  health,
  selected,
  now,
  select,
  start,
  requestBusy = false,
  openSession,
}: {
  card: ProjectCard;
  title: string;
  projectId: string;
  sessions: readonly CardSession[];
  health: Health;
  selected: boolean;
  now: number;
  select: () => void;
  start: (trustedClick: boolean) => void;
  requestBusy?: boolean;
  openSession: (id: string) => void;
}): ReactElement {
  const management = useProjectActions(projectId, title);
  const name = title;
  const handbook = displayName(card);
  const description = card.manifest?.description ?? null;
  const summary =
    handbook === title
      ? description
      : [handbook, description].filter((t) => t !== null).join(" · ");
  const last = Math.max(card.changedMs, ...sessions.map((s) => s.bumpedAt));
  return (
    <Card
      data-item-id={projectId}
      data-testid="projects.project-card-view-pj-card"
      data-project={card.project}
      data-project-id={projectId}
      selected={selected}
      onClick={select}
      onContextMenu={management.onContextMenu}
    >
      <div className="pj-top">
        <div className="pj-heading">
          {management.renameField ?? (
            <button
              data-testid="projects.project-card-view-stop-propagation-2"
              type="button"
              className="pj-name"
              aria-expanded={selected}
              aria-label={`${name}: show the handbook`}
              onClick={(e) => {
                e.stopPropagation();
                select();
              }}
            >
              {name}
              {card.state === "lite" && <Badge presentation="project-label">Markdown</Badge>}
            </button>
          )}
          {management.button}
        </div>
        {summary !== null && <div className="pj-summary">{summary}</div>}
      </div>
      {card.state === "none" ? (
        <div className="pj-nohb">
          <Icon name="book-open" size={16} />
          <span className="pj-nohb-text">No handbook yet</span>
          <Button
            data-testid="projects.project-card-view-stop-propagation-3"
            type="button"
            appearance="project"
            shape="outlined"
            disabled={requestBusy}
            title={requestBusy ? HANDBOOK_BUSY_TOOLTIP : undefined}
            onClick={(e) => {
              e.stopPropagation();
              start(e.nativeEvent.isTrusted);
            }}
          >
            Ask Vibe to create the handbook
          </Button>
        </div>
      ) : card.plans.length === 0 ? (
        <div className="pj-plan">
          <div className="pj-nowstep">No active plan</div>
        </div>
      ) : (
        card.plans.slice(0, 2).map((plan) => <PlanBar key={plan.path} line={planLine(plan)} />)
      )}
      {sessions.length > 0 && (
        <div className="pj-sess">
          {sessions.slice(0, 4).map((s) => (
            <SessionChip key={s.id} session={s} open={openSession} />
          ))}
          {sessions.length > 4 && <span className="pj-more">+{sessions.length - 4}</span>}
        </div>
      )}
      <div className="pj-foot">
        <PathText path={card.project} className="pj-path" />
        {last > 0 && <span className="pj-ago">· {ago(last, now)}</span>}
        {health.text !== null && (
          <span className="pj-health" data-tone={health.tone}>
            {health.tone === "warn" && <Icon name="triangle-alert" size={14} />}
            {health.text}
          </span>
        )}
      </div>
      {management.menu}
    </Card>
  );
}

/** The card of a project without a main folder: its sessions and "Choose main folder…". */
export function NoFolderCard({
  title,
  projectId,
  sessions,
  selected,
  select,
  chooseFolder,
  openSession,
}: {
  title: string;
  projectId: string;
  sessions: readonly CardSession[];
  selected: boolean;
  select: () => void;
  chooseFolder: () => void;
  openSession: (id: string) => void;
}): ReactElement {
  const management = useProjectActions(projectId, title);
  return (
    <Card
      data-item-id={projectId}
      data-testid="projects.project-card-view-pj-card-2"
      data-project=""
      data-project-id={projectId}
      selected={selected}
      onClick={select}
      onContextMenu={management.onContextMenu}
    >
      <div className="pj-top">
        <div className="pj-heading">
          {management.renameField ?? (
            <button
              data-testid="projects.project-card-view-stop-propagation-4"
              type="button"
              className="pj-name"
              aria-expanded={selected}
              aria-label={`${title}: show the details`}
              onClick={(e) => {
                e.stopPropagation();
                select();
              }}
            >
              {title}
            </button>
          )}
          {management.button}
        </div>
      </div>
      <div className="pj-nohb">
        <Icon name="folder" size={16} />
        <span className="pj-nohb-text">No main folder: no handbook to show</span>
        <Button
          data-testid="projects.project-card-view-stop-propagation-5"
          type="button"
          appearance="project"
          shape="outlined"
          onClick={(e) => {
            e.stopPropagation();
            chooseFolder();
          }}
        >
          Choose main folder…
        </Button>
      </div>
      {sessions.length > 0 && (
        <div className="pj-sess">
          {sessions.slice(0, 4).map((s) => (
            <SessionChip key={s.id} session={s} open={openSession} />
          ))}
          {sessions.length > 4 && <span className="pj-more">+{sessions.length - 4}</span>}
        </div>
      )}
      {management.menu}
    </Card>
  );
}
