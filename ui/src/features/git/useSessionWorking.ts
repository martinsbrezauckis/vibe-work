/** Subscribes Git controls to the live chat's work state. Uses the same admission as the Git controller; no polling. */
import { useStore } from "../../state/createStore";
import { sessionWorking } from "../../state/sessionWorking";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

/** True while the chat `sessionId` is sending, compacting or has a turn running; re-renders when that changes. */
export function useSessionWorking(sessionId: string): boolean {
  const { composer } = useApp();
  const sending = useStore(
    composer.store,
    (s) => s.sending[sessionId] === true || s.compacting[sessionId] === true,
  );
  const working = useCoreState((s) => sessionWorking(s, undefined, sessionId));
  return sending || working;
}
