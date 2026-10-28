/**
 * The first-run setup screen for the Mistral key: covers the content area while no working key
 * exists (or while the result of the test is being read). One short step, plain words: it picks the
 * step the key status calls for (`stepOf`) and renders it under a title that says where the user is.
 * Depends on: AppContext, useKeys, the step components, state/keyView, state/keyText, SendToOffer (the one-time
 * right-click menus question under the step).
 * Called by: Content (when `keys.covers()`), Settings (the same steps, inline).
 * Invariants: reads state only; every change goes through the key controller; no secret is kept here.
 */
import { blockedText } from "../../state/keyText";
import { covers } from "../../state/keyStore";
import { stepOf } from "../../state/keyView";
import { installTitle, vibeCovers } from "../../state/vibeInstall";
import { KeyStep } from "./KeyStep";
import { ResultStep } from "./ResultStep";
import { UnlockStep } from "./UnlockStep";
import { VaultStartStep } from "./VaultStartStep";
import { useKeyUi } from "./useKeys";
import { VibeInstallStep, useVibeUi } from "./VibeInstallStep";
import { SendToOffer } from "../shell/SendToOffer";

/** The screen. */
export function SetupScreen() {
  const status = useKeyUi((s) => s.status);
  const pending = useKeyUi((s) => s.resultPending);
  const test = useKeyUi((s) => s.test);
  const step = stepOf(status);
  const showResult = pending || step.kind === "ready";
  // the Vibe install comes first and runs while the user adds the key.
  const installing = useVibeUi(vibeCovers);
  const vibeTitle = useVibeUi((s) => installTitle(s.view));
  const keyCovers = useKeyUi(covers);
  if (!keyCovers) {
    return (
      <section
        className="setup"
        aria-labelledby="setup-title"
        data-step="vibe"
        data-testid="setup-screen"
      >
        <h2 id="setup-title">{vibeTitle}</h2>
        <VibeInstallStep />
        <SendToOffer />
      </section>
    );
  }
  return (
    <section
      className="setup"
      aria-labelledby="setup-title"
      data-testid="setup-screen"
      data-step={showResult ? "result" : step.kind}
    >
      {installing && (
        <>
          <h2>{vibeTitle}</h2>
          <VibeInstallStep />
        </>
      )}
      <h2 id="setup-title">
        {showResult
          ? test.phase === "ok"
            ? "Your key is ready"
            : // the body says the key was refused; the title must not say "Checking".
              test.phase === "failed"
              ? "That key did not work"
              : "Checking your key"
          : step.kind === "unlock"
            ? "Open your vault"
            : step.kind === "create" || step.kind === "pickVault"
              ? "Choose your vault"
              : "Add your Mistral key"}
      </h2>
      {showResult ? (
        <ResultStep />
      ) : step.kind === "create" || step.kind === "pickVault" ? (
        <VaultStartStep />
      ) : step.kind === "unlock" ? (
        <UnlockStep />
      ) : step.kind === "key" ? (
        <KeyStep />
      ) : step.kind === "blocked" ? (
        <p className="key-problem" role="alert">
          {blockedText(step.reason)}
        </p>
      ) : (
        <p>Reading the vault…</p>
      )}
      {/* The one-time question about the right-click menus (macOS and Linux have no installer to ask it). */}
      <SendToOffer />
    </section>
  );
}
