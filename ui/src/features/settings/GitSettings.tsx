/** Git identity in app settings, read and explicitly saved through the core; mounted by SettingsPage. */
import { GithubReadsSettings } from "./GithubReadsSettings";
import { Button } from "../../components/Controls";
import { useEffect } from "react";
import { useStore } from "../../state/createStore";
import { gitIdentityFor, validIdentity } from "../../state/gitIdentity";
import { GitIdentityFields } from "../git/GitIdentityFields";
import { useApp } from "../shell/AppContext";

function IdentitySettings({ identity }: { identity: ReturnType<typeof gitIdentityFor> }) {
  const state = useStore(identity.store, (s) => s);
  useEffect(() => {
    void identity.load();
  }, [identity]);
  const empty = state.draft.name.trim() === "" && state.draft.email.trim() === "";
  return (
    <section aria-labelledby="git-settings-title">
      <h3 id="git-settings-title">Git</h3>
      <div data-setting="git-identity">
        <p className="key-line">
          Name and email for commits made in Vibe Work. Saved in this app’s private data folder.
          Git’s global config is never changed.
        </p>
        {state.view?.source === "global" && (
          <p className="key-line">
            Prefilled from Git’s global config (read only). Save to use these details in Vibe Work.
          </p>
        )}
        <GitIdentityFields
          identity={state.draft}
          onChange={identity.edit}
          disabled={state.busy || state.view === null}
        />
        <p className="key-line">
          Existing repository identities are kept. New repositories save these details locally.
          Leave both fields empty to ask when initializing a repository.
        </p>
        <Button
          type="button"
          data-testid="settings.git-identity-save"
          variant="primary"
          shape="outlined"
          disabled={state.busy || state.view === null || (!empty && !validIdentity(state.draft))}
          onClick={() => void identity.save()}
        >
          {state.busy ? "Saving…" : "Save"}
        </Button>
        {state.saved && (
          <p className="key-line" role="status">
            Saved in Vibe Work.
          </p>
        )}
        {state.error !== null && (
          <p className="key-problem" role="alert">
            {state.error}
          </p>
        )}
      </div>
      <GithubReadsSettings />
    </section>
  );
}

/** The app-wide Git section needs no open session or repository. */
export function GitSettings() {
  const { core } = useApp();
  return core === null ? null : <IdentitySettings identity={gitIdentityFor(core)} />;
}
