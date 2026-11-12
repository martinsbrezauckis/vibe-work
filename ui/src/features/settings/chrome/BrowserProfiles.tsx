/** Individual paired profile rows. Uses the Browser settings command hook; every action carries an immutable ID. */
import { useState } from "react";
import { TEXT } from "../../../vocabulary";
import { Button, SettingsRow, Switch } from "../../../components/Controls";
import type { BrowserProfile, ProfileState } from "../../../protocol/core";
import type { ChromeControlState } from "./useChromeControl";

const states: Record<ProfileState, string> = {
  connected: "Connected",
  offline: "Offline",
  extensionMissing: "Extension missing",
  unavailable: "Unavailable",
};

function ProfileRow({ profile, chrome }: { profile: BrowserProfile; chrome: ChromeControlState }) {
  const [name, setName] = useState(profile.name);
  const busy = chrome.busy !== null;
  const id = profile.profileId;
  const save = () => {
    if (name.trim() !== profile.name)
      void chrome.run({ op: "renameProfile", profileId: id, name: name.trim() }, id);
  };
  return (
    <SettingsRow data-setting="browser-profile" data-item-id={id}>
      <div className="browser-profile-name">
        <div className="key-field-row">
          <input
            data-testid="settings.browser-profile-name"
            data-item-id={id}
            aria-label={`Profile name ${profile.name}`}
            type="text"
            maxLength={80}
            value={name}
            disabled={busy}
            onChange={(event) => {
              setName(event.target.value);
            }}
            onBlur={save}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
              if (event.key === "Escape") setName(profile.name);
            }}
          />
        </div>
        <span className="d" role="status">
          {states[profile.state]}
          {profile.reason ? ` · ${profile.reason}` : ""}
        </span>
      </div>
      <div className="browser-profile-actions">
        <Button
          data-testid="settings.browser-profile-default"
          data-item-id={id}
          aria-label={`${TEXT.modelFallback} ${profile.name}`}
          aria-pressed={profile.isDefault}
          variant={profile.isDefault ? "primary" : undefined}
          shape="outlined"
          disabled={busy || !profile.eligible || profile.isDefault}
          onClick={() => void chrome.run({ op: "setDefaultProfile", profileId: id }, id)}
        >
          {TEXT.modelFallback}
        </Button>
        <Button
          data-testid="settings.browser-profile-connect"
          data-item-id={id}
          shape="outlined"
          disabled={busy || profile.eligible}
          onClick={() => void chrome.run({ op: "connectProfile", profileId: id }, id)}
        >
          {TEXT.connectorConnect}
        </Button>
        <Button
          data-testid="settings.browser-profile-remove"
          data-item-id={id}
          shape="outlined"
          variant="danger"
          disabled={busy}
          onClick={() => void chrome.run({ op: "removeProfile", profileId: id }, id)}
        >
          Remove
        </Button>
        <Switch
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
      </div>
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
  const rows =
    chrome.view?.pairedProfiles?.filter((profile) => profile.browser === browser.toLowerCase()) ??
    [];
  return rows.map((profile) => (
    <ProfileRow key={`${profile.profileId}:${profile.name}`} profile={profile} chrome={chrome} />
  ));
}
