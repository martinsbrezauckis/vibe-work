/**
 * "Install Vibe" on the setup screen: Vibe Work ships no Vibe. This step offers to install the pinned
 * Vibe from Mistral's official channel (PyPI) into Vibe Work's own folder, or to use an install already on this
 * computer. The user sees exactly what will be downloaded (sources, checksums, commands, sizes) before agreeing;
 * then progress with the last output lines and Cancel; a failure says why in plain words, with Try again. After an
 * app update that replaces an older app-owned Vibe the install runs by itself and says which version it replaces.
 * Depends on: AppContext, state/vibeInstall, protocol types. Called by: SetupScreen.
 * Invariants: reads state only; every change goes through the install controller; links open in the system
 * browser through the host's link opener.
 */
import { Button } from "../../components/Controls";

import type { ExistingView, InstallOffer, InstallPhase } from "../../protocol/core";
import { type Store, createStore, useStore } from "../../state/createStore";
import type { VibeUi } from "../../state/vibeInstall";
import { useApp } from "../shell/AppContext";
import { OfferDetails } from "./VibeOfferDetails";

const NONE: Store<VibeUi> = createStore<VibeUi>({
  availability: "none",
  view: null,
  busy: false,
  problem: null,
  dismissed: null,
});

/** Selects from the install state. */
export function useVibeUi<S>(select: (state: VibeUi) => S): S {
  return useStore(useApp().vibe?.store ?? NONE, select);
}

/** What a failure means and what to do, in plain words. */
function failText(phase: Extract<InstallPhase, { state: "failed" }>): string {
  switch (phase.kind) {
    case "network":
      return "Vibe Work could not reach the download server. Check your internet connection or proxy settings, then try again.";
    case "checksum":
      return "A download did not match its published checksum, so it was deleted and not used. Try again; if it happens again, your network may be changing downloads.";
    case "timeout":
      return `"${phase.step}" took too long and was stopped. Try again.`;
    case "cancelled":
      return "The install was cancelled. Nothing was left behind.";
    case "disk":
      return "Vibe Work could not write its install folder (disk space or permissions).";
    case "start":
      return "Vibe was installed but did not start.";
    default:
      return `"${phase.step}" did not finish.`;
  }
}

function Offer({ offer }: { offer: InstallOffer }) {
  const { vibe } = useApp();
  const busy = useVibeUi((s) => s.busy);
  return (
    <div className="vibe-offer" data-testid="vibe-offer">
      <p>
        Vibe Work will install <strong>Vibe {offer.vibeVersion}</strong> with Python {offer.python}{" "}
        into its own folder. Nothing outside that folder changes: your PATH and other tools stay as
        they are.
      </p>
      <OfferDetails offer={offer} />
      <div className="key-actions">
        <Button
          data-testid="setup.vibe-install-step-confirm"
          type="button"
          appearance="dialog"
          disabled={busy}
          onClick={() => void vibe?.confirm(offer.offerId)}
        >
          Install Vibe {offer.vibeVersion}
        </Button>
        <Button
          data-testid="setup.vibe-install-step-cancel"
          type="button"

          disabled={busy}
          onClick={() => void vibe?.cancel()}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

function Existing({ found }: { found: readonly ExistingView[] }) {
  const { vibe } = useApp();
  const busy = useVibeUi((s) => s.busy);
  if (found.length === 0) return <p>No Vibe install was found on this computer.</p>;
  return (
    <ul className="vibe-existing">
      {found.map((item) => (
        <li data-item-id={item.path} key={item.path}>
          <span>
            <code>{item.path}</code>{" "}
            {item.version === null ? "(did not answer)" : `version ${item.version}`}
          </span>
          {item.warning !== null && <p className="key-problem">{item.warning}</p>}
          {item.supported && (
            <div className="key-actions">
              <Button
                data-testid="setup.vibe-install-step-use-existing"
                type="button"

                disabled={busy}
                onClick={() => void vibe?.useExisting(item.path)}
              >
                Use this one
              </Button>
            </div>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The step. */
export function VibeInstallStep() {
  const { vibe } = useApp();
  const view = useVibeUi((s) => s.view);
  const busy = useVibeUi((s) => s.busy);
  const problem = useVibeUi((s) => s.problem);
  if (view === null) return null;
  const phase = view.phase;
  return (
    <div className="setup-step vibe-install" data-vibe={phase.state} aria-live="polite">
      {phase.state === "notInstalled" && (
        <>
          <p>
            Vibe Work runs Mistral&apos;s Vibe, which is not installed for it yet. It is installed
            from Mistral&apos;s official package on PyPI, only after you agree.
          </p>
          <div className="key-actions">
            <Button
              data-testid="setup.vibe-install-step-prepare"
              type="button"
              appearance="dialog"
              disabled={busy}
              onClick={() => void vibe?.prepare()}
            >
              Review the install
            </Button>
            <Button
              data-testid="setup.vibe-install-step-check-existing"
              type="button"

              disabled={busy}
              onClick={() => void vibe?.checkExisting()}
            >
              Use Vibe already on this computer
            </Button>
          </div>
        </>
      )}
      {phase.state === "offered" && <Offer offer={phase.offer} />}
      {phase.state === "installing" && (
        <>
          {view.replacing !== undefined && (
            <p className="vibe-replacing">
              Vibe Work now uses Vibe {view.pinnedVersion}. Vibe {view.replacing} is being replaced;
              your chats and settings stay.
            </p>
          )}
          <p role="status">
            Step {phase.index} of {phase.total}: {phase.step} ({Math.round(phase.elapsedMs / 1000)}{" "}
            s)
          </p>
          <progress max={phase.total} value={phase.index - 1} />
          {phase.outputTail !== "" && (
            <pre className="vibe-tail">{phase.outputTail.split("\n").slice(-12).join("\n")}</pre>
          )}
          <Button
            data-testid="setup.vibe-install-step-cancel-2"
            type="button"

            onClick={() => void vibe?.cancel()}
          >
            Cancel
          </Button>
        </>
      )}
      {phase.state === "failed" && (
        <>
          <p className="key-problem" role="alert">
            {failText(phase)}
          </p>
          <details>
            <summary data-testid="setup.vibe-install-step-summary">Details</summary>
            <pre className="vibe-tail">
              {[phase.error, phase.outputTail].filter((t) => t !== "").join("\n")}
            </pre>
          </details>
          <Button
            data-testid="setup.vibe-install-step-prepare-2"
            type="button"
            appearance="dialog"
            disabled={busy}
            onClick={() => void vibe?.prepare()}
          >
            Try again
          </Button>
        </>
      )}
      {phase.state === "installed" && phase.warning !== null && (
        <p className="key-problem">{phase.warning}</p>
      )}
      {view.checkedExisting && phase.state !== "installed" && phase.state !== "installing" && (
        <Existing found={view.existing} />
      )}
      {problem !== null && (
        <p className="key-problem" role="alert">
          {problem.text}
        </p>
      )}
    </div>
  );
}
