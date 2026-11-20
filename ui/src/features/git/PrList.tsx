/**
 * The Git panel's "Pull requests" list: every pull request of the chat's branches (its folder's and its copy's),
 * newest first, with number, title, state and the status line with its cause. Each row opens to show its
 * checks, review and mergeability; its link opens the forge page.
 * Depends on: state/git (`list`), model.ts. Called by: BranchBar.
 * Invariants: GitHub is asked only while the list is open: once when it opens, on Refresh, and every
 * `CARD_POLL_MS` while it stays open and the window is visible (the core keeps its own floor). Closed, it asks
 * nothing. Forge texts are rendered as text; only `https:` links open, through the core's `openLink`.
 */
import { type ReactElement, useEffect, useState } from "react";

import { Button } from "../../components/Controls";
import type { PrCard } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import { CARD_POLL_MS, type Git } from "../../state/git";
import { useApp } from "../shell/AppContext";
import {
  checksText,
  fetchedText,
  httpsLink,
  mergeText,
  reviewText,
  stateText,
  statusNotes,
  statusText,
} from "./model";

/** The list for `sessionId`, closed until the person opens it. */
export function PrList({ git, sessionId }: { git: Git; sessionId: string }): ReactElement {
  const { core } = useApp();
  const [open, setOpen] = useState(false);
  const [shown, setShown] = useState<number | null>(null);
  const list = useStore(git.store, (s) => s.lists[sessionId]);
  useEffect(() => {
    if (!open) return;
    void git.list(sessionId, true);
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void git.list(sessionId, true);
    }, CARD_POLL_MS);
    return () => {
      clearInterval(timer);
    };
  }, [git, sessionId, open]);
  const visit = (card: PrCard) => {
    const link = httpsLink(card.url);
    if (link !== null) core?.client.openLink(link.url).catch(() => undefined);
  };
  return (
    <div className="pr-list" data-git="pr-list">
      <Button
        data-testid="git.pr-list-toggle"
        type="button"
        appearance="compact"
        aria-expanded={open}
        onClick={() => {
          setOpen((o) => !o);
        }}
      >
        Pull requests
      </Button>
      {open && (
        <>
          {list?.cards.length === 0 && (
            <p className="cmuted">No pull requests for this chat's branches.</p>
          )}
          <ul className="pr-rows">
            {list?.cards.map((card) => (
              <li key={card.number} data-pr-number={card.number} data-item-id={String(card.number)}>
                <Button
                  data-testid="git.pr-list-row"
                  type="button"
                  appearance="link"
                  aria-expanded={shown === card.number}
                  onClick={() => {
                    setShown((n) => (n === card.number ? null : card.number));
                  }}
                >
                  #{card.number} {card.title}
                </Button>{" "}
                <span className={`pr-state ${card.state}`}>{stateText(card)}</span>
                {statusText(card) !== null && (
                  <span className="pr-cause"> · {statusText(card)}</span>
                )}
                {shown === card.number && (
                  <ul className="pr-lines" data-git="pr-list-card">
                    {statusNotes(card).map((text) => (
                      <li key={text}>{text}</li>
                    ))}
                    <li>{checksText(card)}</li>
                    <li>{reviewText(card)}</li>
                    {mergeText(card) !== "" && <li>{mergeText(card)}</li>}
                    <li>
                      <Button
                        data-testid="git.pr-list-open"
                        type="button"
                        appearance="link"
                        title={httpsLink(card.url)?.host ?? "not https"}
                        onClick={() => {
                          visit(card);
                        }}
                      >
                        Open on GitHub
                      </Button>
                    </li>
                  </ul>
                )}
              </li>
            ))}
          </ul>
          <div className="pr-actions">
            <Button
              data-testid="git.pr-list-refresh"
              type="button"
              appearance="compact"
              onClick={() => void git.list(sessionId, true)}
            >
              Refresh
            </Button>
            {list !== undefined && (
              <span className="pr-when">
                {list.note ?? fetchedText({ ...list, card: null, openUrl: null })}
              </span>
            )}
          </div>
        </>
      )}
    </div>
  );
}
