/** Deleting a chat while Windows still has one of its files open: the person sees one short, plain sentence, not
 * Vibe's raw exception text (that stays in the app's log, where the core records every failed command). Other
 * delete failures keep the core's message. Synthetic data. */
import { describe, expect, it } from "vitest";
import { CoreCallError } from "../../../ui/src/protocol/client/types";
import type { CoreError } from "../../../ui/src/protocol/core";
import { initialCore } from "../../../ui/src/state/coreModel";
import type { CoreController } from "../../../ui/src/state/coreStore";
import { createStore } from "../../../ui/src/state/createStore";
import { createSessionActions } from "../../../ui/src/state/sessionActions";
import type { Shell } from "../../../ui/src/state/shellStore";
import { createToasts } from "../../../ui/src/state/toastStore";

const IN_USE =
  "session/delete failed: [WinError 32] The process cannot access the file because it is being used by another " +
  "process: 'C:\\\\Users\\\\someone\\\\.vibe\\\\logs\\\\session\\\\unified\\\\s1\\\\meta.json' (internal_error)";

/** Deletes chat "s1" against a core whose `session/delete` fails with `error`; returns the toasts shown. */
async function deleteFailing(error: CoreError) {
  const store = createStore({
    ...initialCore,
    sessions: {
      s1: {
        id: "s1",
        session: { id: "s1", title: "Notes" },
        localOnly: false,
        openCallbackIds: [],
      },
    },
  });
  const core = {
    store,
    run: (command: { type: string }) =>
      command.type === "requestConfirmation"
        ? Promise.resolve({ type: "confirmation", token: "t1" })
        : Promise.reject(new CoreCallError(error)),
    forget: () => undefined,
  } as unknown as CoreController;
  const shell = {
    store: createStore({ selected: "s1" }),
    actions: { selectSession: () => undefined },
  } as unknown as Shell;
  const toasts = createToasts();
  const deleted = await createSessionActions({ core, shell, toasts }).remove(
    "s1",
  );
  return { deleted, texts: toasts.store.get().map((t) => t.text) };
}

describe("a chat that cannot be deleted because a file is in use", () => {
  it("shows one plain sentence instead of Vibe's raw text", async () => {
    const { deleted, texts } = await deleteFailing({
      code: "serverError",
      message: IN_USE,
      method: "session/delete",
      summary: null,
      serverCode: "internal_error",
    });
    expect(deleted).toBe(false);
    expect(texts).toEqual([
      "This chat could not be deleted because Windows still has one of its files open. Try again in a moment.",
    ]);
  });

  it("keeps the core's message for any other delete failure", async () => {
    const { texts } = await deleteFailing({
      code: "serverError",
      message: "session/delete failed: session not found (not_found)",
      method: "session/delete",
      summary: null,
      serverCode: "not_found",
    });
    expect(texts).toEqual([
      "session/delete failed: session not found (not_found)",
    ]);
  });
});
