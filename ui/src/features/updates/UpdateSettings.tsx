/**
 * Settings, "Updates": the running version, what the update flow is doing, the switch "Download
 * updates automatically" (on by default; off = Vibe Work asks before it downloads) and the buttons the state
 * allows. Renders nothing in a build without an update channel.
 * a "Vibe" row shows the running Vibe and what the same check found in the signed supported-Vibe list,
 * with Update when a tested newer version can be installed (the consent dialog follows).
 * Depends on: AppContext, state/updates, state/vibeUpdates, ReleaseHost. Called by: settings/SettingsPage (the Settings
 * dialog is gone).
 * Invariants: the switch only saves the preference (the host stores it); installing is the same "Install and
 * restart" request as the notice's, so it is never offered before the download is verified. A package-manager
 * install (`linkOnly`) gets no download switch: nothing is downloaded there.
 */
import { Button, Switch } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import { statusLine } from "../../state/updates";
import { vibeStatusLine } from "../../state/vibeUpdates";
import { useVibeUi } from "../setup/VibeInstallStep";
import { useApp } from "../shell/AppContext";
import { ReleaseHost } from "./ReleaseHost";

/** The section. */
export function UpdateSettings() {
  const { updates } = useApp();
  if (updates === null) return null;
  return <Section updates={updates} />;
}

function Section({ updates }: { updates: NonNullable<ReturnType<typeof useApp>["updates"]> }) {
  const state = useStore(updates.store, (s) => s);
  const view = state.view;
  if (!state.available || view === null) return null;
  const phase = view.phase;
  const busy = ["checking", "downloading", "installing", "restarting"].includes(phase.type);
  const run = (step: () => Promise<void>) => () => {
    step().catch(() => undefined);
  };
  return (
    <section data-testid="update-settings">
      <h3>Updates</h3>
      <div className="check" data-setting="update-status">
        <span>
          Status
          <span className="d" data-testid="update-status">
            {statusLine(view)}
          </span>
          {view.mode === "linkOnly" && (
            <span className="d">
              Vibe Work tells you about new versions but does not install them in this copy.
              Download the new version from the release page and install it the way you installed
              this one.
            </span>
          )}
        </span>
        <span className="set-actions">
          {phase.type === "available" && view.mode === "inApp" && (
            <Button
              data-testid="updates.update-settings-run"
              type="button"
              shape="outlined"
              onClick={run(updates.download)}
            >
              Download
            </Button>
          )}
          {phase.type === "ready" && (
            <Button
              data-testid="updates.update-settings-run-2"
              type="button"
              shape="outlined"
              variant="primary"
              onClick={run(updates.install)}
            >
              Install and restart
            </Button>
          )}
          {phase.type === "available" && view.mode === "linkOnly" && view.releasePage !== null && (
            <>
              <Button
                data-testid="updates.update-settings-run-3"
                type="button"
                shape="outlined"
                onClick={run(updates.openReleasePage)}
              >
                Open release page
              </Button>
              <ReleaseHost url={view.releasePage} />
            </>
          )}
          <Button
            data-testid="updates.update-settings-run-4"
            type="button"
            shape="outlined"
            disabled={busy}
            onClick={run(updates.check)}
          >
            Check now
          </Button>
        </span>
      </div>
      <VibeRow />
      {view.mode === "inApp" && (
        <div className="check" data-setting="auto-update">
          <span>
            Download updates automatically
            <span className="d">
              On: a new version downloads in the background and Vibe Work then asks before it
              installs. Off: Vibe Work asks before it downloads. Either way you get a notice, and
              nothing installs or restarts without your click.
            </span>
          </span>
          <Switch
            data-testid="updates.update-settings-run-5"
            aria-label="Download updates automatically"
            aria-checked={view.autoDownload}
            onClick={run(() => updates.setAutoDownload(!view.autoDownload))}
          />
        </div>
      )}
    </section>
  );
}

/** The Vibe row; nothing in a run that does not install Vibe (a development build). */
function VibeRow() {
  const { vibe } = useApp();
  const view = useVibeUi((s) => s.view);
  const busy = useVibeUi((s) => s.busy);
  if (vibe === null || view === null) return null;
  const phase = view.update.phase.state;
  const canUpdate = view.update.available !== null && (phase === "idle" || phase === "failed");
  return (
    <div className="check" data-setting="vibe-update">
      <span>
        Vibe
        <span className="d" data-testid="vibe-update-status">
          {vibeStatusLine(view)}
        </span>
      </span>
      {canUpdate && (
        <span className="set-actions">
          <Button
            data-testid="updates.update-settings-prepare-update"
            type="button"
            shape="outlined"
            variant="primary"
            disabled={busy}
            onClick={() => void vibe.prepareUpdate()}
          >
            Update Vibe
          </Button>
        </span>
      )}
    </div>
  );
}
