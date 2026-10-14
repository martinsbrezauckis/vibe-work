/**
 * One core-backed snapshot for the vault's cryptographic lock and the agent's secrets/MCP pause.
 * Shared by the Vault header, status bar and Account controls; core events refresh the snapshot.
 */
import { useCallback, useEffect } from "react";

import { toCallError } from "../../protocol/client/types";
import { createStore, useStore, type Store } from "../../state/createStore";
import type { CoreController } from "../../state/coreStore";
import { vaultReadiness } from "../../state/keyReadiness";
import { problem, secrets } from "../../state/secrets";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "../shell/AppContext";
import { goHome } from "../shell/home";

type Access = { unlocked: boolean; agentAccessLocked: boolean };
type Snapshot = { access: Access | null; busy: boolean };
type AccessStore = {
  state: Store<Snapshot>;
  refresh: () => Promise<void>;
  watch: () => () => void;
};
const EMPTY = createStore<Snapshot>({ access: null, busy: false });
const STORES = new WeakMap<CoreController, AccessStore>();

function accessStore(core: CoreController): AccessStore {
  const existing = STORES.get(core);
  if (existing) return existing;
  const state = createStore<Snapshot>({ access: null, busy: false });
  let watchers = 0;
  let unlisten: (() => void) | null = null;
  let serial = 0;
  const refresh = () => {
    const request = ++serial;
    return secrets(core, { op: "list" })
      .then((result) => {
        if (request === serial && result.kind === "list")
          state.set((s) => ({
            ...s,
            access: { unlocked: result.unlocked, agentAccessLocked: result.agentAccessLocked },
          }));
      })
      .catch(() => {
        if (request === serial) state.set((s) => ({ ...s, access: null }));
      });
  };
  const watch = () => {
    watchers++;
    if (watchers === 1) {
      void refresh();
      unlisten = core.listen((event) => {
        if (
          event.type === "secretsChanged" ||
          event.type === "keyStatusChanged" ||
          event.type === "lagged"
        )
          void refresh();
      });
    }
    return () => {
      watchers--;
      if (watchers === 0) {
        unlisten?.();
        unlisten = null;
        serial++;
      }
    };
  };
  const shared = { state, refresh, watch };
  STORES.set(core, shared);
  return shared;
}

/** Two separate lock actions, backed by the same authoritative core snapshot. */
export function useVaultAccess() {
  const app = useApp();
  const { core, keys, shell, toasts } = app;
  const shared = core === null ? null : accessStore(core);
  const snapshot = useStore(shared?.state ?? EMPTY, (s) => s);
  const keyState = useKeyUi((s) => s.status?.vaultState ?? null);
  const blocked = useKeyUi(vaultReadiness);
  const keyBusy = useKeyUi((s) => s.busy);
  useEffect(() => shared?.watch(), [shared]);

  const vaultLocked =
    keyState === "locked" || snapshot.access?.unlocked === false || blocked !== null;
  const agentPaused = snapshot.access?.agentAccessLocked ?? null;
  const openUnlock = useCallback(() => {
    goHome(app);
    if (blocked?.destination === "setup") keys?.reopen();
    else shell.actions.openSettings("account", "mistral-key");
  }, [app, blocked, keys, shell]);
  const toggleAgent = useCallback(() => {
    if (core === null || snapshot.busy || keyBusy) return;
    // A selected closed vault can be revived by the core's remembered-key path in this click.
    // Setup and unavailable states have no selected vault to unlock.
    if (blocked !== null && !(blocked.kind === "locked" && keyState === "locked")) {
      openUnlock();
      return;
    }
    if (snapshot.access === null && keyState !== "locked") return;
    shared?.state.set((s) => ({ ...s, busy: true }));
    void secrets(core, {
      op: "setAgentAccess",
      locked: keyState === "locked" || vaultLocked ? false : !snapshot.access?.agentAccessLocked,
    })
      .then(async () => {
        if (keyState === "locked") await keys?.load();
        await shared?.refresh();
      })
      .catch((thrown: unknown) => {
        if (toCallError(thrown).error.serverCode === "passwordRequired") {
          openUnlock();
          void keys?.load();
        } else toasts.push({ kind: "error", bellKind: "failure", text: problem(thrown) });
        void shared?.refresh();
      })
      .finally(() => shared?.state.set((s) => ({ ...s, busy: false })));
  }, [core, snapshot, keyBusy, keyState, vaultLocked, blocked, openUnlock, shared, keys, toasts]);
  const toggleVault = useCallback(() => {
    if (keyBusy || keys === null) return;
    if (keyState !== "unlocked" || vaultLocked) {
      openUnlock();
      return;
    }
    void keys.lockNow().then(() => shared?.refresh());
  }, [keyBusy, keyState, vaultLocked, openUnlock, keys, shared]);

  return {
    vaultLocked,
    agentPaused,
    unknown: snapshot.access === null,
    vaultAvailable: keys !== null,
    busy: snapshot.busy || keyBusy,
    toggleAgent,
    toggleVault,
  };
}
