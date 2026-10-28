/**
 * Settings › Safety, section "How much Vibe may do without asking": one read-only row per level
 * with a link to the one place it is set, then the summary line `data-setting="ask-allowed"` ("You have allowed 214
 * things without asking", the three counts, Review → Settings › Allowed without asking) in its four states (reading,
 * none, vault locked, a list unreadable). No controls here. Renders only.
 * Depends on: components/Controls, settings/vibeSettingRows (the Vibe settings read), allowed/levels, useAllowed,
 * allowed/open. Called by: SettingsPage (Safety).
 */
import type { ReactElement } from "react";

import { LinkButton, SettingsRow } from "../../../components/Controls";
import { useApp } from "../../shell/AppContext";
import { useVibeSettings } from "../vibeSettingRows";
import { askFirstLine, chromeAvailable, safetyLine } from "../chrome/chromeRows";
import { useChromeControl } from "../chrome/useChromeControl";
import { accessValue, closeTabsValue, levelRows, openLevel, useBrowserPermissions } from "./levels";
import { openAllowed } from "./open";
import { useAllowed } from "./useAllowed";

/** The section. */
export function AllowedLevels(): ReactElement | null {
  const app = useApp();
  const vibe = useVibeSettings();
  const browser = useBrowserPermissions();
  const chrome = useChromeControl();
  const { summary } = useAllowed();
  if (app.core === null) return null;
  const access = accessValue({
    running: vibe.running,
    via: vibe.via,
    mode: vibe.view === null ? undefined : vibe.view.defaultAgent.value,
    error: vibe.error,
  });
  return (
    <section aria-labelledby="allowed-levels-title" className="allow-levels">
      <h3 id="allowed-levels-title">How much Vibe may do without asking</h3>
      {levelRows(
        access,
        closeTabsValue(browser.permissions, browser.error),
        safetyLine(chrome.view),
        askFirstLine(chrome.view),
        chromeAvailable(chrome.view),
      ).map((row) => (
        <SettingsRow key={row.id} data-level={row.id}>
          <span>
            {row.label}
            <span className="d">{row.value}</span>
          </span>
          <LinkButton
            data-testid="settings.allowed-level-link"
            data-item-id={row.id}
            onClick={() => {
              openLevel(app, row.id);
            }}
          >
            {row.link}
          </LinkButton>
        </SettingsRow>
      ))}
      <SettingsRow data-setting="ask-allowed" data-state={summary.state}>
        <span>
          {summary.title}
          {summary.detail && (
            <span className="d" role="status">
              {summary.detail}
            </span>
          )}
        </span>
        {summary.review && (
          <LinkButton
            data-testid="settings.allowed-review"
            onClick={() => {
              openAllowed(app);
            }}
          >
            Review
          </LinkButton>
        )}
      </SettingsRow>
    </section>
  );
}
