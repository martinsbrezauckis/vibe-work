/**
 * A browser's paired profiles in Browser settings. Names are the browser's own (its profile picker's name, the
 * signed-in account under it); nothing here edits a name. One profile needs no row: the browser card already says
 * whether it is connected. With several, a dropdown picks the default for new chats ("Ask in each chat" clears it),
 * and one compact row per profile keeps its state, its own labelled Use switch and Remove. A single profile that is Off or needs attention keeps its row so it
 * can be turned back on. Remove asks first (`ProfileRemoveDialog`). Every action carries the immutable profile ID.
 * Uses the Browser settings command hook.
 */
import { useState } from "react";
import { TEXT } from "../../../vocabulary";
import { Button, SettingsRow, Switch } from "../../../components/Controls";
import { Selector } from "../../../components/Selector";
import type { BrowserProfile, ProfileState } from "../../../protocol/core";
import type { ChromeControlState } from "./useChromeControl";
import { ProfileRemoveDialog } from "./ChromeDialogs";

const states: Record<ProfileState, string> = {
  connected: "Connected",
  offline: "Offline",
  extensionMissing: "Extension missing",
  unavailable: "Unavailable",
};

/**
 * "work@example.com · Connected": the account (when signed in and not already the name) and the profile's state.
 * Same-named profiles signed in to different accounts are told apart by this account; the core adds the pairing
 * time to the name only when nothing else differs. An offline row adds why ("Offline · Extension off in this
 * profile"); another row's reason stands in for its state ("Off").
 */
function detail(profile: BrowserProfile): string {
  const account =
    profile.account?.toLowerCase() === profile.name.toLowerCase() ? undefined : profile.account;
  const state =
    profile.state === "offline" && profile.reason !== undefined
      ? `${states.offline} · ${profile.reason}`
      : (profile.reason ?? states[profile.state]);
  return [account, state].filter((part) => part !== undefined && part !== "").join(" · ");
}

function ProfileRow({
  profile,
  chrome,
  onRemove,
}: {
  profile: BrowserProfile;
  chrome: ChromeControlState;
  onRemove: (profile: BrowserProfile) => void;
}) {
  const busy = chrome.busy !== null;
  const id = profile.profileId;
  const switchId = `browser-profile-use-${id}`;
  return (
    <SettingsRow data-setting="browser-profile" data-item-id={id}>
      <span className="browser-profile-name">
        {profile.name}
        <span className="d" role="status">
          {detail(profile)}
        </span>
      </span>
      <span className="browser-profile-actions">
        <Button
          data-testid="settings.browser-profile-remove"
          data-item-id={id}
          aria-label={`Remove ${profile.name}`}
          appearance="compact"
          variant="danger"
          disabled={busy}
          onClick={() => {
            onRemove(profile);
          }}
        >
          Remove
        </Button>
        <label className="browser-profile-use" htmlFor={switchId}>
          {TEXT.browserProfileUse}
        </label>
        <Switch
          id={switchId}
          data-testid="settings.browser-profile-enabled"
          data-item-id={id}
          aria-label={`Use ${profile.name}`}
          aria-checked={profile.enabled}
          disabled={busy}
          onClick={() =>
            void chrome.run(
              { op: "setProfileEnabled", profileId: id, enabled: !profile.enabled },
              id,
            )
          }
        />
      </span>
    </SettingsRow>
  );
}

export function BrowserProfiles({
  browser,
  chrome,
}: {
  browser: string;
  chrome: ChromeControlState;
}) {
  const [removing, setRemoving] = useState<BrowserProfile | null>(null);
  const rows =
    chrome.view?.pairedProfiles?.filter((profile) => profile.browser === browser.toLowerCase()) ??
    [];
  const only = rows.length === 1 ? rows[0] : undefined;
  const dialog = (
    <ProfileRemoveDialog
      profile={removing}
      chrome={chrome}
      browser={browser}
      onClose={() => {
        setRemoving(null);
      }}
    />
  );
  if (rows.length === 0 || (only?.enabled === true && only.reason === undefined)) return dialog;
  if (only !== undefined)
    return (
      <>
        <ProfileRow profile={only} chrome={chrome} onRemove={setRemoving} />
        {dialog}
      </>
    );
  const chosen = rows.find((profile) => profile.isDefault);
  return (
    <>
      <SettingsRow data-setting="browser-profile-default">
        <span>
          {TEXT.browserProfileDefault}
          <span className="d">{TEXT.browserProfileDefaultHint}</span>
        </span>
        <Selector
          label={`${browser} ${TEXT.browserProfileDefault.toLowerCase()}`}
          value={chosen?.profileId ?? ""}
          valueLabel={chosen?.name ?? TEXT.browserProfileAsk}
          options={[
            { value: "", label: TEXT.browserProfileAsk },
            ...rows.map((profile) => ({
              value: profile.profileId,
              label: profile.name,
              description: detail(profile),
              disabled: !profile.eligible,
            })),
          ]}
          disabled={chrome.busy !== null}
          compact
          align="end"
          testId="settings.browser-profile-default"
          onChange={(profileId) => {
            // "Ask in each chat" clears the current default; choosing a profile makes it the default.
            if (profileId !== "")
              void chrome.run({ op: "setDefaultProfile", profileId }, profileId);
            else if (chosen !== undefined)
              void chrome.run(
                { op: "clearDefaultProfile", profileId: chosen.profileId },
                chosen.profileId,
              );
          }}
        />
      </SettingsRow>
      {rows.map((profile) => (
        <ProfileRow
          key={profile.profileId}
          profile={profile}
          chrome={chrome}
          onRemove={setRemoving}
        />
      ))}
      {dialog}
    </>
  );
}
