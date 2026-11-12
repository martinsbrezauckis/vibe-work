/** The chat's fixed browser profile and person-only selection. Core owns defaults, release acknowledgement and errors. */
import { composerShown, type ComposerBrowser } from "./browserTabs";
import { useState } from "react";
import { Selector } from "../../components/Selector";
import { NEW_CHAT } from "../../state/newChatKey";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";
import { selectBrowserProfile } from "./profileSelection";

const names: Record<string, ComposerBrowser> = {
  chrome: "Chrome",
  edge: "Edge",
  safari: "Safari",
  firefox: "Firefox",
};
export function BrowserProfilePicker({ sessionId }: { sessionId: string }) {
  const app = useApp();
  const view = useCoreState((state) => state.chromeControl);
  const draft = useShellState((state) => state.newChatOwner);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const owner = sessionId === NEW_CHAT ? draft : sessionId;
  const binding = view?.chatProfiles?.find((row) => row.sessionId === owner);
  const selected =
    binding && composerShown(view, names[binding.profileId?.split(":")[0] ?? ""] ?? "Chrome")
      ? binding
      : undefined;
  const profiles = (view?.pairedProfiles ?? []).filter((row) =>
    composerShown(view, names[row.browser] ?? "Chrome"),
  );
  const eligible = profiles.filter((row) => row.eligible);
  if (profiles.length === 0 && selected === undefined) return null;
  const label = (row: (typeof profiles)[number]) =>
    `${names[row.browser] ?? row.browser} · ${row.name}`;
  const only = eligible.length === 1 ? eligible[0] : undefined;
  const single =
    only !== undefined &&
    (binding === undefined ||
      (selected?.profileId === only.profileId && selected.state === "selected"));
  if (single)
    return (
      <span className="composer-profile-label" data-testid="composer.browser-profile-selected">
        {selected?.name ?? label(only)}
      </span>
    );
  const state =
    selected?.state === "unconfirmed"
      ? "Unconfirmed"
      : selected?.state === "stopping"
        ? "Stopping"
        : selected?.state === "removed"
          ? // The browser deleted this profile (and its extension); core keeps the chat bound until the person moves it.
            `${selected.name} was removed. Choose another profile or reconnect this profile.`
          : undefined;
  return (
    <div className="composer-profile-choice">
      <Selector
        label="Browser profile"
        value={selected?.profileId ?? ""}
        valueLabel={selected?.name ?? "Choose profile"}
        options={profiles.map((row) => ({
          value: row.profileId,
          label: label(row),
          disabled: !row.eligible,
          description:
            row.reason ??
            (row.eligible ? "Connected" : row.state === "offline" ? "Offline" : "Unavailable"),
        }))}
        disabled={busy}
        compact
        testId="composer.browser-profile"
        {...(state === undefined ? {} : { ariaDescription: state })}
        onChange={(profileId) => {
          setBusy(true);
          setError(null);
          void selectBrowserProfile(app, sessionId, profileId)
            .catch((reason: unknown) => {
              setError(
                reason instanceof Error
                  ? reason.message
                  : "Selection unconfirmed. Reconnect the old profile and retry.",
              );
            })
            .finally(() => {
              setBusy(false);
            });
        }}
      />
      {state !== undefined && <span role="status">{state}</span>}
      {error !== null && <span role="alert">{error}</span>}
    </div>
  );
}
