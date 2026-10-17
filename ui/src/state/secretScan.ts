/** Session-log scan UI state and typed actions. Shell wiring starts it; Settings and bell consume metadata only. */
import type { SecretScanRequest, SecretScanView } from "../protocol/core";
import { toCallError } from "../protocol/client/types";
import type { CoreController } from "./coreStore";
import { createStore } from "./createStore";
import { findingText } from "./secretScanText";
import type { Toasts } from "./toastStore";

/** Keep a single security notice per chat and let Rust supply every finding transition. */
export function createSecretScan(core: CoreController, toasts: Toasts) {
  const store = createStore<{
    view: SecretScanView | null;
    pending: readonly string[];
    error: string | null;
  }>({ view: null, pending: [], error: null });
  const seen = new Map<string, string>();
  let revision = 0;
  const accept = (view: SecretScanView) => {
    store.set((state) => ({ ...state, view }));
    for (const finding of view.findings) {
      const text = findingText(finding);
      const signature = JSON.stringify(finding);
      if (seen.get(finding.findingId) === signature) continue;
      seen.set(finding.findingId, signature);
      const spec = {
        text,
        secretLeak: { findingId: finding.findingId },
      };
      const old = toasts.history
        .get()
        .find((notice) => notice.secretLeak?.findingId === finding.findingId);
      if (old) toasts.update(old.id, { kind: "info", bellKind: "security", ...spec });
      else toasts.record({ kind: "info", bellKind: "security", ...spec });
    }
  };
  const run = async (request: SecretScanRequest): Promise<void> => {
    const key = request.op === "redact" ? request.findingId : "settings";
    if (store.get().pending.includes(key)) return;
    store.set((state) => ({ ...state, pending: [...state.pending, key], error: null }));
    try {
      const result = await core.run({ type: "secretScan", request });
      if (result.type === "secretScan") accept(result.view);
    } catch (thrown) {
      store.set((state) => ({ ...state, error: toCallError(thrown).error.message }));
    } finally {
      store.set((state) => ({ ...state, pending: state.pending.filter((item) => item !== key) }));
    }
  };
  return {
    store,
    run,
    /** Subscribe before the initial read; a newer event takes precedence over an older read. */
    start(): () => void {
      let live = true;
      const read = () => {
        const started = revision;
        void core
          .run({ type: "secretScan", request: { op: "read" } })
          .then((result) => {
            if (live && revision === started && result.type === "secretScan") accept(result.view);
          })
          .catch(() => {});
      };
      const stop = core.listen((event) => {
        if (event.type === "secretScan") {
          revision++;
          accept(event.view);
        }
        if (
          event.type === "lagged" ||
          (event.type === "server" && event.status.state === "running")
        )
          read();
      });
      read();
      return () => {
        live = false;
        stop();
      };
    },
  };
}

/** Scanner UI controller, absent without a core connection. */
export type SecretScan = ReturnType<typeof createSecretScan>;
