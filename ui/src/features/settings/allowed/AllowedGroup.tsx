/**
 * One group card of the Allowed without asking page: a RowDisclosure header with the name and
 * a count Badge, the bulk link in the header while open, the help line, the rows or the sub-groups (each with its own
 * header, "Open session" for a session, its bulk link), the read problem, the locked vault line and the group's
 * status line. Renders only; every action is the store's.
 * Depends on: components/Controls, components/Icon, allowed/store, view, AllowedRows. Called by: AllowedPage.
 */
import type { ReactElement } from "react";

import { Badge, LinkButton, RowDisclosure } from "../../../components/Controls";
import { Icon } from "../../../components/Icon";
import { term } from "../../../vocabulary";
import { AllowedRows } from "./AllowedRows";
import type { Allowed, AllowedState } from "./store";
import { type GroupView, type LeafView, VAULT_LOCKED_PAGE } from "./view";

/** The disclosure header of a group (h3) or a sub-group (h4). */
function Header(props: {
  level: 3 | 4;
  focusKey: string;
  title: string;
  count: number | null;
  open: boolean;
  onToggle: () => void;
  controls: string;
}): ReactElement {
  const H = props.level === 3 ? "h3" : "h4";
  return (
    <H className="allow-title">
      <RowDisclosure
        data-testid="settings.allowed-toggle"
        data-item-id={props.focusKey}
        data-allowed-focus={`header:${props.focusKey}`}
        aria-expanded={props.open}
        aria-controls={props.controls}
        onClick={props.onToggle}
      >
        <Icon name={props.open ? "chevron-down" : "chevron-right"} size={14} />
        <span>{props.title}</span>
        {props.count !== null && <Badge data-count="">{props.count}</Badge>}
      </RowDisclosure>
    </H>
  );
}

/** A bulk link ("Revoke all 46…" / "Revoke these 17…"). */
function Bulk({ leaf, allowed }: { leaf: LeafView; allowed: Allowed }): ReactElement | null {
  if (leaf.bulk === null) return null;
  return (
    <LinkButton
      data-testid="settings.allowed-revoke-all"
      data-item-id={leaf.key}
      data-allowed-focus={`bulk:${leaf.key}`}
      variant="danger"
      onClick={() => {
        allowed.askRevokeAll(leaf.key);
      }}
    >
      {leaf.bulk}
    </LinkButton>
  );
}

function bodyId(key: string): string {
  return `allowed-body-${key.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
}

/** A sub-group: a session's answers, or one vault part. */
function Subgroup({
  leaf,
  allowed,
  busy,
}: {
  leaf: LeafView;
  allowed: Allowed;
  busy: AllowedState["busy"];
}): ReactElement {
  const id = bodyId(leaf.key);
  return (
    <div className="allow-sub" data-leaf={leaf.key}>
      <div className="allow-head">
        <Header
          level={4}
          focusKey={leaf.key}
          title={leaf.title}
          count={leaf.entries.length}
          open={leaf.open}
          controls={id}
          onToggle={() => {
            allowed.toggle(leaf.key);
          }}
        />
        {leaf.sessionId !== null && (
          <LinkButton
            data-testid="settings.allowed-open-session"
            data-item-id={leaf.key}
            onClick={() => {
              if (leaf.sessionId !== null) void allowed.openSession(leaf.sessionId);
            }}
          >
            {term("workNames", "session").tooltip}
          </LinkButton>
        )}
        <Bulk leaf={leaf} allowed={allowed} />
      </div>
      {leaf.open && (
        <div className="allow-body" id={id}>
          <AllowedRows leaf={leaf} allowed={allowed} busy={busy} />
        </div>
      )}
    </div>
  );
}

/** A group card. */
export function AllowedGroup({
  group,
  allowed,
  state,
}: {
  group: GroupView;
  allowed: Allowed;
  state: AllowedState;
}): ReactElement {
  const id = bodyId(group.id);
  const unreadable = group.problem !== null || group.locked;
  const status = state.status[group.id];
  const refused = state.problem[group.id];
  return (
    <section
      className="allow-group"
      data-group={group.id}
      data-setting={`allowed-${group.id}`}
      aria-label={group.title}
    >
      <div className="allow-head">
        <Header
          level={3}
          focusKey={group.id}
          title={group.title}
          count={unreadable ? null : group.count}
          open={group.open}
          controls={id}
          onToggle={() => {
            allowed.toggle(group.id);
          }}
        />
        {group.leaf !== null && <Bulk leaf={group.leaf} allowed={allowed} />}
      </div>
      {group.problem !== null && (
        <p className="key-problem" role="alert">
          {group.problem}
        </p>
      )}
      {group.locked && (
        <div className="allow-locked">
          <p className="key-line">{VAULT_LOCKED_PAGE}</p>
          <LinkButton
            data-testid="settings.allowed-open-secrets"
            onClick={() => {
              allowed.openProjectSecrets();
            }}
          >
            Open Project secrets
          </LinkButton>
        </div>
      )}
      {group.open && !unreadable && (
        <div className="allow-body" id={id}>
          <p className="ui-help">{group.help}</p>
          {group.note !== null && <p className="ui-help">{group.note}</p>}
          {group.leaf !== null && (
            <AllowedRows leaf={group.leaf} allowed={allowed} busy={state.busy} />
          )}
          {group.subgroups.map((s) => (
            <Subgroup key={s.key} leaf={s} allowed={allowed} busy={state.busy} />
          ))}
        </div>
      )}
      {refused !== undefined && (
        <p className="key-problem" role="alert">
          {refused}
        </p>
      )}
      <p className="key-line allow-status" role="status">
        {status ?? ""}
      </p>
    </section>
  );
}
