/**
 * Inline key or vault notices and their single recovery action, also used by the status bar.
 * Depends on: AppContext, useKeyUi, keyReadiness. Called by: Content, Composer, StatusBar.
 * Invariants: reads metadata only; recovery uses the existing setup or Account screens; copy and
 * send guards react to the same key state without remounting the chat or clearing its draft.
 */
import { keyReadiness, vaultReadiness } from "../../state/keyReadiness";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "./AppContext";
import { goHome } from "./home";

/** The current reason the agent cannot answer; `null` when a key can be used. */
export function useKeyReadiness(requirement: "key" | "vault" = "key") {
  return useKeyUi(requirement === "vault" ? vaultReadiness : keyReadiness);
}

/** One feature-specific sentence and one existing recovery action; disappears live on recovery. */
export function KeyNotice({
  message,
  requirement = "key",
  inSettings = false,
}: {
  message?: string;
  requirement?: "key" | "vault";
  /** Settings notices use the same row, helper and primary button classes as their page. */
  inSettings?: boolean;
}) {
  const app = useApp();
  const { keys, shell } = app;
  const state = useKeyReadiness(requirement);
  if (state === null) return null;
  const visibleMessage = state.kind === "locked" ? state.message : (message ?? state.message);
  return (
    <section
      className={inSettings ? "check" : "key-notice"}
      aria-label={requirement === "vault" ? "Vault required" : "Mistral key required"}
      aria-live="polite"
    >
      {inSettings ? (
        <span className="key-field-hint">{visibleMessage}</span>
      ) : (
        <p>{visibleMessage}</p>
      )}
      <button
        type="button"
        className={inSettings ? "tb-btn primary" : "btn-primary"}
        onClick={() => {
          goHome(app);
          if (state.destination === "setup") keys?.reopen();
          else shell.actions.openSettings("account", "mistral-key");
        }}
      >
        {state.action}
      </button>
    </section>
  );
}
