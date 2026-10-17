/** Visible child approval rows shared by the transcript renderer and its UI-control route. */
import { subagentCardsOf, type CoreState } from "../../../state/coreModel";
import type { JsonValue } from "../../../protocol/core";
import type { Row } from "./build";

/** Only the parent's children currently blocked on the matching callback contribute rows. */
export function subagentRows(
  cards: CoreState["subagentCards"],
  sessionId: string,
  childSessions: JsonValue | undefined,
): Row[] {
  return subagentCardsOf(cards, sessionId, childSessions).flatMap((card): Row[] =>
    card.entry !== null && typeof card.entry === "object" && !Array.isArray(card.entry)
      ? [
          {
            kind: "callback",
            key: `sub:${card.callbackId}`,
            entry: card.entry,
            related: null,
            answerFor: card.sessionId,
          },
        ]
      : [],
  );
}
