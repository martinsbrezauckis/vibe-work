/**
 * The PR card under the branch bar: state, checks, review, mergeable, a
 * refresh, the forge page, and "Send failing checks to Vibe" / "Send review comments to Vibe", which put a
 * prepared message into the composer (never sent). Without a known PR, a pushed GitHub branch offers "Check for
 * a pull request".
 * Depends on: state/git, model.ts. Called by: BranchBar.
 * Invariants: the forge is asked only on a press, and every `CARD_POLL_MS` while this card is shown, a card
 * exists and the window is visible (the core keeps its own floor). Links come from forge and CI data (a third
 * party): only `https:` ones open, in the system browser through the core's `openLink` (checked again in
 * Rust), with their host shown; anything else says "link not opened (not https)".
 */
import { type ReactElement, useEffect, useState } from "react";

import { useStore } from "../../state/createStore";
import { CARD_POLL_MS, type Git } from "../../state/git";
import { useApp } from "../shell/AppContext";
import { checksText, fetchedText, httpsLink, mergeText, reviewText, stateText } from "./model";

/** The card for `sessionId`. */
export function PrCard({ git, sessionId }: { git: Git; sessionId: string }): ReactElement | null {
  const { core } = useApp();
  const view = useStore(git.store, (s) => s.cards[sessionId]);
  const status = useStore(git.store, (s) => s.status[sessionId]);
  const busy = useStore(git.store, (s) => s.busy[sessionId] === true);
  const card = view?.card ?? status?.pr ?? null;
  const hasCard = card !== null;
  useEffect(() => {
    if (!hasCard) return;
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void git.card(sessionId, true);
    }, CARD_POLL_MS);
    return () => {
      clearInterval(timer);
    };
  }, [git, sessionId, hasCard]);
  const github = status?.state?.remotes.some((r) => r.forge?.kind === "gitHub") ?? false;
  const pushed = status?.state?.upstream != null;
  const [refused, setRefused] = useState<string | null>(null);
  /** Opens a forge or CI address when it is https, else says why not. */
  const open = (text: string | null | undefined) => {
    const link = httpsLink(text);
    if (link === null) {
      setRefused("link not opened (not https)");
      return;
    }
    setRefused(null);
    core?.client.openLink(link.url).catch(() => undefined);
  };
  /** The host of an address for the label (empty when it is not https). */
  const host = (text: string | null | undefined) => httpsLink(text)?.host ?? "not https";
  if (card === null) {
    if (!github || !pushed) return null;
    return (
      <div className="pr-card pr-none" data-git="pr-card">
        <span>{view?.note ?? "No pull request known for this branch."}</span>
        <button
          data-testid="git.pr-card-card"
          type="button"
          className="mini"
          disabled={busy}
          data-action="pr-refresh"
          onClick={() => void git.card(sessionId, true)}
        >
          Check for a pull request
        </button>
        {view?.openUrl != null && (
          <button
            data-testid="git.pr-card-open"
            type="button"
            className="key-link"
            title={host(view.openUrl)}
            onClick={() => {
              open(view.openUrl);
            }}
          >
            Open on GitHub
          </button>
        )}
      </div>
    );
  }
  const failing = card.checks.items.filter((i) => i.outcome !== "pending");
  return (
    <div className={`pr-card pr-${card.state}`} data-git="pr-card" data-pr-number={card.number}>
      <div className="pr-head">
        <span className={`pr-state ${card.state}`}>{stateText(card)}</span>
        <button
          data-testid="git.pr-card-open-2"
          type="button"
          className="key-link pr-title"
          title={host(card.url)}
          onClick={() => {
            open(card.url);
          }}
        >
          #{card.number} {card.title}
        </button>
      </div>
      <ul className="pr-lines">
        <li
          className={
            card.checks.failed > 0
              ? "bad"
              : card.checks.pending > 0
                ? "wait"
                : card.checks.passed > 0
                  ? "ok"
                  : ""
          }
        >
          {checksText(card)}
        </li>
        <li>{reviewText(card)}</li>
        {mergeText(card) !== "" && (
          <li className={card.mergeable === "conflicting" ? "bad" : ""}>{mergeText(card)}</li>
        )}
      </ul>
      {failing.length > 0 && (
        <ul className="pr-failing">
          {failing.slice(0, 5).map((item) => (
            <li data-item-id={item.name} key={item.name}>
              {item.url === null ? (
                item.name
              ) : (
                <button
                  data-testid="git.pr-card-open-3"
                  type="button"
                  className="key-link"
                  title={host(item.url)}
                  onClick={() => {
                    open(item.url);
                  }}
                >
                  {item.name}
                </button>
              )}{" "}
              ({item.outcome}
              {item.url === null ? "" : `, ${host(item.url)}`})
            </li>
          ))}
        </ul>
      )}
      {refused !== null && (
        <p className="key-problem" role="alert">
          {refused}
        </p>
      )}
      <div className="pr-actions">
        {card.checks.failed > 0 && (
          <button
            data-testid="git.pr-card-prepare"
            type="button"
            className="mini"
            disabled={busy}
            data-action="pr-send-checks"
            onClick={() => void git.prepare(sessionId, "checks")}
          >
            Send failing checks to Vibe
          </button>
        )}
        {card.state === "open" && (
          <button
            data-testid="git.pr-card-prepare-2"
            type="button"
            className="mini"
            disabled={busy}
            data-action="pr-send-review"
            onClick={() => void git.prepare(sessionId, "review")}
          >
            Send review comments to Vibe
          </button>
        )}
        <button
          data-testid="git.pr-card-card-2"
          type="button"
          className="mini"
          disabled={busy}
          data-action="pr-refresh"
          onClick={() => void git.card(sessionId, true)}
        >
          Refresh
        </button>
        {view !== undefined && <span className="pr-when">{view.note ?? fetchedText(view)}</span>}
      </div>
    </div>
  );
}
