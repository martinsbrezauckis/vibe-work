/** GitHub reader row under the Git identity. Depends on core-owned state and shared controls. */
import { SettingsRow, Switch } from "../../components/Controls";
import type { GithubReadsView } from "../../protocol/core";
import { useGithubReads } from "../../state/useGithubReads";
import { useApp } from "../shell/AppContext";

/** Exact account and lifecycle status for Settings and its live region. */
function githubReadsStatus(view: GithubReadsView | null, pending: boolean): string {
  if (pending || view === null || view.status === "starting") return "Starting…";
  switch (view.status) {
    case "missing":
      return "gh is not installed.";
    case "signedOut":
      return "gh is not signed in: run gh auth login in a terminal.";
    case "signedIn":
      return `gh is signed in as ${view.login ?? ""}.`;
    case "on":
      return "On: new chats can read GitHub.";
    case "error":
      return `Could not start the GitHub reader: ${view.reason ?? "the reader is unavailable"}.`;
    case "enterprise":
      return "Only github.com is supported.";
    case "loggedOut":
      return "gh signed out, so GitHub reads stopped.";
  }
}

/** Opt-in account reads. The token stays outside UI state. */
export function GithubReadsSettings() {
  const { core } = useApp();
  const { view, pending, error, change } = useGithubReads(core);
  if (core === null) return null;
  const unavailable =
    view === null ||
    ["missing", "signedOut", "enterprise", "loggedOut", "starting"].includes(view.status);
  return (
    <div data-setting="gh-reads">
      <SettingsRow>
        <span>
          <label htmlFor="github-reads-switch">Let Vibe read GitHub with your gh login</label>
          <span className="d" id="github-reads-description">
            Read pull requests, issues, runs and the GitHub API with your gh login. Vibe never sees
            your token and cannot write.
          </span>
          <span className="d" id="github-reads-status" aria-live="polite">
            {githubReadsStatus(view, pending)}
          </span>
        </span>
        <Switch
          id="github-reads-switch"
          data-testid="settings.github-reads-enabled"
          aria-describedby="github-reads-description github-reads-status"
          aria-checked={view?.enabled ?? false}
          disabled={pending || unavailable}
          onClick={() => {
            void change(!(view?.enabled ?? false));
          }}
        />
      </SettingsRow>
      {(error !== null || view?.status === "error") && (
        <p className="key-problem" role="alert">
          {error === null
            ? githubReadsStatus(view, false)
            : `Could not start the GitHub reader: ${error}.`}
        </p>
      )}
    </div>
  );
}
