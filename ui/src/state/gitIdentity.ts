/** App commit identity drafts and explicit saves through the typed core; called by Git Settings. */
import type { CommitIdentity, GitIdentitySettings } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";

/** Both author fields must be present and safe for a Git author line; email shape is only a warning. */
export function validIdentity(identity: CommitIdentity): boolean {
  return [identity.name, identity.email].every(
    (value) => value.trim() !== "" && !/[\p{Cc}<>]/u.test(value),
  );
}

/** A deliberately permissive email hint; never controls Save or Commit. */
export function unusualEmail(email: string): boolean {
  return email.trim() !== "" && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());
}

function createIdentity(core: CoreController) {
  const store = createStore({
    draft: { name: "", email: "" },
    view: null as GitIdentitySettings | null,
    busy: false,
    error: null as string | null,
    saved: false,
  });
  const request = async (identity?: CommitIdentity) => {
    store.set((s) => ({ ...s, busy: true, error: null, saved: false }));
    try {
      const result = await core.run({
        type: "git",
        request: identity === undefined ? { op: "identity" } : { op: "setIdentity", identity },
      });
      if (result.type !== "git" || result.result.kind !== "identity")
        throw new Error("Unexpected Git identity response.");
      const view = result.result.settings;
      store.set((s) => ({ ...s, view, draft: view.identity, saved: identity !== undefined }));
    } catch (thrown) {
      store.set((s) => ({ ...s, error: toCallError(thrown).error.message }));
    } finally {
      store.set((s) => ({ ...s, busy: false }));
    }
  };
  return {
    store,
    load: () => request(),
    edit: (draft: CommitIdentity) => {
      store.set((s) => ({ ...s, draft, saved: false }));
    },
    save: () => request(store.get().draft),
  };
}

const identities = new WeakMap<CoreController, ReturnType<typeof createIdentity>>();

/** One settings draft per core; repository flows use only the core's saved value. */
export function gitIdentityFor(core: CoreController) {
  let identity = identities.get(core);
  if (identity === undefined) {
    identity = createIdentity(core);
    identities.set(core, identity);
  }
  return identity;
}
