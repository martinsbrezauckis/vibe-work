/** Opens an OS notice's current chat destination and waiting prompt. Host wiring calls it; missing chats use retained bell history. */
import type { HostEvent } from "../../protocol/client/types";
import { goToWaitingPrompt, promptsOf } from "../ask/waiting";
import { goHome } from "../shell/home";
import type { App } from "../shell/AppContext";

/** A resolved chat owns activation; only an unavailable destination opens the bell. */
export async function activateNotification(
  app: App,
  event: Extract<HostEvent, { type: "openSession" }>,
): Promise<void> {
  const { core, sessions, shell, toasts } = app;
  if (core === null || sessions === null) return;
  const retained = toasts.history.get().find((notice) => notice.id === event.notificationId);
  // A child's notice may predate its hosting card; use the current destination at click time.
  const card = Object.values(core.store.get().subagentCards).find(
    (card) => card.sessionId === event.sessionId,
  );
  const sessionId =
    card?.parentSessionId ?? (retained === undefined ? event.sessionId : retained.sessionId);
  const fallback = () => {
    if (retained !== undefined) toasts.open(retained.id);
    else if (sessionId) toasts.openSession(sessionId);
    else toasts.open();
  };
  if (!sessionId) {
    fallback();
    return;
  }
  let entry = core.store.get().sessions[sessionId];
  if (entry === undefined || (entry.state === null && !entry.localOnly)) {
    try {
      await core.run({ type: "refreshSession", sessionId });
    } catch {
      fallback();
      return;
    }
    entry = core.store.get().sessions[sessionId];
  }
  if (entry === undefined || (entry.state === null && !entry.localOnly)) {
    fallback();
    return;
  }
  toasts.close();
  goHome(app);
  shell.actions.unfoldChat();
  await sessions.open(sessionId);
  if (core.store.get().sessions[sessionId] === undefined) {
    fallback();
    return;
  }
  const notice =
    retained ??
    (event.notificationId === undefined
      ? [...toasts.history.get()].reverse().find((row) => row.sessionId === sessionId)
      : undefined);
  if (notice !== undefined) toasts.markRead(notice.id);
  if (promptsOf(core.store.get(), sessionId).length > 0) goToWaitingPrompt(app, sessionId);
}
