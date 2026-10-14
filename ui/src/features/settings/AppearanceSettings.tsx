/**
 * Settings › Appearance: theme, status usage, and transcript thinking disclosure.
 * Depends on: AppContext, hooks, components/Selector. Called by: SettingsPage.
 */
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { switchTheme } from "../../state/themeSwitch";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";

/** The Appearance section. */
export function AppearanceSettings(): ReactElement {
  const { shell } = useApp();
  const theme = useShellState((s) => s.theme);
  const showUsage = useShellState((s) => s.showUsage);
  const showThinkingExpanded = useShellState((s) => s.showThinkingExpanded);
  return (
    <section aria-labelledby="appearance-title">
      <h3 id="appearance-title">Appearance</h3>
      <div className="check" data-setting="theme">
        <span>
          Theme
          <span className="d">
            Both themes are complete; the title bar switch flips between them.
          </span>
        </span>
        <Selector
          label="Theme"
          value={theme}
          options={[
            { value: "dark", label: "Dark", icon: "moon" },
            { value: "light", label: "Light", icon: "sun" },
          ]}
          onChange={(t) => {
            switchTheme(t, shell.actions.setTheme);
          }}
        />
      </div>
      <div className="check" data-setting="status-usage">
        <span>
          Show usage in the status bar
          <span className="d">
            How full the open conversation&apos;s context is, as &quot;Context 42%&quot;. A click
            opens Account and usage.
          </span>
        </span>
        <button
          type="button"
          className="switch"
          role="switch"
          aria-label="Show usage in the status bar"
          aria-checked={showUsage}
          onClick={() => {
            shell.actions.setShowUsage(!showUsage);
          }}
        />
      </div>
      <h4>Transcript</h4>
      <div className="check" data-setting="show-thinking-expanded">
        <span>
          Show thinking expanded
          <span className="d">
            Open thinking in the conversation by default. You can still close each block yourself.
          </span>
        </span>
        <button
          type="button"
          className="switch"
          role="switch"
          aria-label="Show thinking expanded"
          aria-checked={showThinkingExpanded}
          onClick={() => {
            shell.actions.setShowThinkingExpanded(!showThinkingExpanded);
          }}
        />
      </div>
    </section>
  );
}
