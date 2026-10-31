/**
 * The core's question for the person's Git name and email (`gitIdentityAsked`): the agent's `git init` or
 * `git commit` waits until it is answered. Save sends `answerIdentity` with the name and email, Cancel without; the
 * question closes on the core's `gitIdentityAskClosed`. The open question lives in the core store (`identityAsk`), so a
 * reloaded window, whose first events replay it, shows it too.
 * Depends on: coreStore (events, commands), createStore, gitIdentity (validation). Called by: GitIdentityAskDialog.
 */
import type { CommitIdentity } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

/** The open question, if any. */
export interface IdentityAskState {
  askId: string | null;
  draft: CommitIdentity;
  busy: boolean;
  error: string | null;
}

function createAsk(core: CoreController) {
  const store = createStore<IdentityAskState>({
    askId: null,
    draft: { name: "", email: "" },
    busy: false,
    error: null,
  });
  const follow = () => {
    const open = core.store.get().identityAsk?.askId ?? null;
    if (open === store.get().askId) return;
    store.set((s) =>
      open === null
        ? { ...s, askId: null, busy: false }
        : { askId: open, draft: { name: "", email: "" }, busy: false, error: null },
    );
  };
  follow();
  core.store.subscribe(follow);
  const send = async (identity: CommitIdentity | null) => {
    const askId = store.get().askId;
    if (askId === null) return;
    store.set((s) => ({ ...s, busy: true, error: null }));
    try {
      await core.run({ type: "git", request: { op: "answerIdentity", askId, identity } });
    } catch (thrown) {
      store.set((s) => ({ ...s, error: toCallError(thrown).error.message }));
    } finally {
      store.set((s) => ({ ...s, busy: false }));
    }
  };
  return {
    store,
    edit: (draft: CommitIdentity) => {
      store.set((s) => ({ ...s, draft }));
    },
    save: () => send(store.get().draft),
    cancel: () => send(null),
  };
}

/** The question's controller. */
export type IdentityAsk = ReturnType<typeof createAsk>;

const asks = new WeakMap<CoreController, IdentityAsk>();

/** One controller per core. */
export function gitIdentityAskFor(core: CoreController): IdentityAsk {
  let ask = asks.get(core);
  if (ask === undefined) {
    ask = createAsk(core);
    asks.set(core, ask);
  }
  return ask;
}
