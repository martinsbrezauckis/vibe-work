/**
 * The chat's card for sites whose request to this computer or the local network was refused because it did not say
 * which open page sent it, or because the site's own question was not answered: one row per site of this chat with
 * Allow and Block. The card's reason says which ({@link cardReason}). Nothing is guessed and nothing is
 * allowed by default; an answer here is the same decision as the native question and Settings > Browser
 * ("Sites not answered yet"). The card reads the core's list (`localNetworkSites`, its `unanswered` sites of this
 * chat) when it mounts and whenever the answers change (`browserChanged` naming no chat), so it closes once every site
 * is answered, wherever the answer was given, and a reloaded window shows it again.
 * Depends on: AskCard, useWho, the typed core, vocabulary. Called by: Transcript (above the conversation).
 */
import { type ReactElement, useEffect, useState } from "react";

import { Button } from "../../components/Controls";
import { ACT_ALLOW, ACT_BLOCK } from "../settings/actionLabels";
import type { SiteAnswer, UnansweredSite } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import type { CoreController } from "../../state/coreStore";
import { TEXT, wording } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { AskCard } from "./AskCard";
import { useWho } from "./useWho";

/** This chat's sites waiting for an answer, from the core's list. */
async function waitingSites(core: CoreController, sessionId: string): Promise<UnansweredSite[]> {
  const reply = await core.run({ type: "browser", request: { op: "localNetworkSites" } });
  return reply.type === "browser" && reply.result.type === "localNetworkSites"
    ? reply.result.unanswered.filter((u) => u.sessionId === sessionId)
    : [];
}

/**
 * Why the card's sites were refused: a request that named no open site, a site's own question that was not
 * answered (naming those sites), or both sentences when the card holds both kinds.
 */
function cardReason(sites: readonly UnansweredSite[]): string {
  const asked = sites.filter((s) => s.asked).map((s) => s.site);
  if (asked.length === 0) return TEXT.localNetworkCardReason;
  const askedLine = wording(TEXT.localNetworkCardReasonAsked, { sites: asked.join(", ") });
  return asked.length === sites.length ? askedLine : `${TEXT.localNetworkCardReason} ${askedLine}`;
}

/** Renders the card for `sessionId`, or nothing while none of its sites waits for an answer. */
export function LocalNetworkAsk({ sessionId }: { sessionId: string }): ReactElement | null {
  const { core } = useApp();
  const who = useWho("vibeWork", sessionId, sessionId);
  const [sites, setSites] = useState<UnansweredSite[]>([]);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    if (core === null) return;
    let live = true;
    const load = () => {
      waitingSites(core, sessionId)
        .then((found) => {
          if (live) setSites(found);
        })
        .catch(() => {
          // Settings > Browser still lists the sites; a failed read shows no card.
        });
    };
    load();
    const stop = core.listen((event) => {
      if (event.type === "browserChanged" && event.sessionId === null) load();
    });
    return () => {
      live = false;
      stop();
    };
  }, [core, sessionId]);
  if (core === null || sites.length === 0) return null;
  const answer = (site: string, value: SiteAnswer) => {
    setBusy(true);
    setProblem(null);
    void core
      .run({ type: "browser", request: { op: "setLocalNetworkSite", site, answer: value } })
      .then(() => waitingSites(core, sessionId))
      .then(setSites)
      .catch((thrown: unknown) => {
        setProblem(toCallError(thrown).error.message);
      })
      .finally(() => {
        setBusy(false);
      });
  };
  return (
    <AskCard
      id={`local-network:${sessionId}`}
      className="local-network-ask"
      question={TEXT.localNetworkCardQuestion}
      who={who.text}
      whoTitle={who.title}
      reason={cardReason(sites)}
      buttons={[]}
      disabled={busy}
      state={problem ?? ""}
      stateRole={problem === null ? "status" : "alert"}
      data={{ "data-entry-id": `local-network:${sessionId}` }}
    >
      <ul className="local-network-sites" aria-label={TEXT.localNetworkCardQuestion}>
        {sites.map(({ site }) => (
          <li key={site} data-item-id={site} data-testid="ask.local-network.site">
            <span className="summary">{site}</span>
            <Button
              shape="outlined"
              disabled={busy}
              aria-label={`${ACT_ALLOW} ${site}`}
              data-testid="ask.local-network.allow"
              onClick={() => {
                answer(site, "allow");
              }}
            >
              {ACT_ALLOW}
            </Button>
            <Button
              shape="outlined"
              disabled={busy}
              aria-label={`${ACT_BLOCK} ${site}`}
              data-testid="ask.local-network.block"
              onClick={() => {
                answer(site, "block");
              }}
            >
              {ACT_BLOCK}
            </Button>
          </li>
        ))}
      </ul>
    </AskCard>
  );
}
