/**
 * The session header's "{n} allowed for this session" chip and its list: every "Allow for
 * this session" answer the core keeps for this session and its subagents, in the card's covers words, each with
 * Revoke, and the link to everything allowed without asking. Hidden while the session has none. Same mechanism and
 * look as the location chip (`LocationChip`): a `Badge` button with `aria-expanded`, the content below the header
 * line.
 * Depends on: Badge, Button, LinkButton, Icon, grantsModel, the core store, composer focus. Called by: the session
 * header (`features/shell/Content.tsx`).
 * Focus: Enter/Space toggles; Escape collapses and returns to the chip; after a Revoke focus goes to the next
 * row's Revoke, else the previous, else the composer text box (the chip is gone with the last grant).
 */
import { type KeyboardEvent, type ReactElement, useEffect, useId, useRef } from "react";

import { Badge } from "../../components/Badge";
import { Button, LinkButton } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import { requestComposerFocus } from "../../state/composerFocus";
import { openAllowed } from "../settings/allowed/open";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useTranscriptUi } from "../transcript/hooks";
import { chipText, childIds, childName, grantDetail, grantWords, grantsOf } from "./grantsModel";

/** The transcript-store key of the list's open state (also what the `ui/transcript` route toggles). */
export const grantsKey = (sessionId: string): string => `grants:${sessionId}`;

/** Renders the chip and, when expanded, the list. */
export function GrantsChip({ sessionId }: { sessionId: string }): ReactElement | null {
  const app = useApp();
  const all = useCoreState((s) => s.askGrants.grants);
  const children = useCoreState((s) => s.sessions[sessionId]?.state?.["childSessions"]);
  const expanded = useTranscriptUi((s) => s.open[grantsKey(sessionId)] === true);
  const busy = useTranscriptUi((s) => s.grantBusy[sessionId] ?? null);
  const status = useTranscriptUi((s) => s.grantStatus[sessionId] ?? "");
  const id = useId();
  const chip = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const grants = grantsOf(all, sessionId, childIds(children));
  // The last grant gone closes the list too: a grant kept later shows the closed chip, not an open list.
  const none = grants.length === 0;
  useEffect(() => {
    if (none && expanded) app.transcript.toggle(grantsKey(sessionId), false);
  }, [none, expanded, app.transcript, sessionId]);
  if (none) return null;

  const revoke = async (grantId: string, index: number) => {
    if (!(await app.transcript.revokeGrant(sessionId, grantId))) return;
    // The list re-renders from the core's event: focus the next row's Revoke, else the previous, else the composer.
    requestAnimationFrame(() => {
      const buttons = list.current?.querySelectorAll<HTMLButtonElement>("[data-grant-revoke]");
      const next = buttons?.[Math.min(index, (buttons.length || 1) - 1)];
      if (next !== undefined) next.focus();
      else requestComposerFocus(sessionId);
    });
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape" || !expanded) return;
    event.preventDefault();
    app.transcript.toggle(grantsKey(sessionId), false);
    chip.current?.focus();
  };
  const subagentName = (session: string): string | null =>
    session === sessionId ? null : (childName(children, session) ?? "subagent");
  return (
    <div className="ask-grants" data-testid="ask.grants" onKeyDown={onKey}>
      <Badge
        ref={chip}
        data-testid="ask.grants-toggle"
        type="button"
        presentation="worktree"
        as="button"
        aria-expanded={expanded}
        aria-controls={id}
        title={chipText(grants.length)}
        data-grants="chip"
        onClick={() => {
          app.transcript.toggle(grantsKey(sessionId), !expanded);
        }}
      >
        <Icon name="list-checks" size={12} />
        <span className="ask-grants-text">{chipText(grants.length)}</span>
        <Icon name={expanded ? "chevron-up" : "chevron-down"} size={12} />
      </Badge>
      <div id={id} ref={list} className="ask-grants-list" hidden={!expanded}>
        <p className="ui-help">Vibe Work answers these again for you until this session ends.</p>
        {grants.map((grant, index) => (
          <div key={grant.id} className="check ask-grant" data-grant-id={grant.id}>
            <span>
              {grantWords(grant).map((part, i) =>
                part.code ? <code key={i}>{part.text}</code> : part.text,
              )}
              <span className="d">{grantDetail(grant, subagentName(grant.session))}</span>
            </span>
            <Button
              data-testid="ask.grants-revoke"
              data-item-id={grant.id}
              data-grant-revoke=""
              type="button"
              shape="outlined"
              disabled={busy !== null}
              aria-label={`Revoke: ${grantWords(grant)
                .map((p) => p.text)
                .join("")}`}
              onClick={() => {
                void revoke(grant.id, index);
              }}
            >
              {busy === grant.id ? "Revoking…" : "Revoke"}
            </Button>
          </div>
        ))}
        <p className="key-line" role="status">
          {status}
        </p>
        <LinkButton
          data-testid="ask.grants-see-all"
          type="button"
          onClick={() => {
            openAllowed(app, { sessionId });
          }}
        >
          See everything allowed without asking
        </LinkButton>
      </div>
    </div>
  );
}
