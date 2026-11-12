/** Person-owned profile changes use the typed core command; draft ownership follows the first message. */
import type { App } from "../shell/AppContext";
import { NEW_CHAT } from "../../state/newChatKey";

export async function selectBrowserProfile(
  app: App,
  sessionId: string,
  profileId: string,
): Promise<void> {
  if (sessionId === NEW_CHAT && app.shell.store.get().newChatOwner === null)
    await app.newChat?.start();
  const owner = sessionId === NEW_CHAT ? app.shell.store.get().newChatOwner : sessionId;
  if (owner === null)
    throw new Error("This chat is not ready. Open the chat and select its profile again.");
  const result = await app.core?.run(
    { type: "chromeControl", request: { op: "selectProfile", sessionId: owner, profileId } },
    { origin: "window" },
  );
  if (result?.type !== "chromeControl")
    throw new Error(
      "The profile selection is unconfirmed. Reconnect the old profile and retry the selection.",
    );
}
