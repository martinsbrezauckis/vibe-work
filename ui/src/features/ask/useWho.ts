/**
 * Who asks, in words, from the stores: "Vibe · “{session}”", "Vibe's subagent “{name}” ·
 * “{session}”", "Vibe in your Chrome · …", "Vibe Work · …". The session title is always named (subagent cards and
 * cards seen after switching sessions need it); `title` holds it in full for the tooltip.
 * Depends on: the core store, model.ts. Called by: the ask adapters.
 */
import { sessionTitle } from "../../state/sessionSummary";
import { arr, messageText, obj, roleOf, turnIdOf } from "../transcript/model/entries";
import { useCoreState } from "../shell/hooks";
import { childName } from "./grantsModel";
import { whoAsks } from "./model";

/** Who asks for a card in `hostSessionId`'s transcript (answered for `sessionId`). */
export function useWho(
  requester: "agent" | "subagent" | "chrome" | "vibeWork",
  hostSessionId: string,
  sessionId: string,
  askingTurnId?: string | null,
): { text: string; title: string } {
  const title = useCoreState((s) => {
    const entry = s.sessions[hostSessionId];
    let asked = "";
    if (askingTurnId !== undefined && askingTurnId !== null)
      for (const raw of arr(entry?.state?.["history"])) {
        const value = obj(raw);
        if (value !== null && roleOf(value) === "user" && turnIdOf(value) === askingTurnId) {
          const message = messageText(value).trim();
          if (message !== "") asked = message;
        }
      }
    return asked || sessionTitle(entry?.session);
  });
  const name = useCoreState((s) =>
    requester === "subagent"
      ? childName(s.sessions[hostSessionId]?.state?.["childSessions"], sessionId)
      : null,
  );
  return { text: whoAsks(requester, title, name ?? undefined), title };
}
