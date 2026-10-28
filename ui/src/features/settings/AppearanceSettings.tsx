/**
 * Settings › Appearance: theme, status usage, open files (tabs, marks on Vibe's lines) and transcript thinking
 * disclosure.
 * Depends on: AppContext, hooks, components/Selector. Called by: SettingsPage.
 */
import { Switch } from "../../components/Controls";
import type { ReactElement } from "react";

import { Selector } from "../../components/Selector";
import { switchTheme } from "../../state/themeSwitch";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";

/** The Appearance section. */
export function AppearanceSettings(): ReactElement {
  const { shell } = useApp();
  const theme = useShellState((s) => s.theme);
  const openFilesIn = useShellState((s) => s.openFilesIn);
  const markVibeLines = useShellState((s) => s.markVibeLines);
  const openAs = useShellState((s) => s.openHtmlMarkdownAs);
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
          testId="settings.appearance-settings-switch-theme"
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
            How full the open session&apos;s context is, as &quot;Context 42%&quot;. A click opens
            Account and usage.
          </span>
        </span>
        <Switch
          data-testid="settings.appearance-settings-set-show-usage"
          aria-label="Show usage in the status bar"
          aria-checked={showUsage}
          onClick={() => {
            shell.actions.setShowUsage(!showUsage);
          }}
        />
      </div>
      <div className="check" data-setting="open-files-in">
        <span>Open files in</span>
        <Selector
          testId="settings.appearance-open-files-in"
          label="Open files in"
          value={openFilesIn}
          options={[
            { value: "tabs", label: "Separate tabs (default)" },
            { value: "single", label: "One File tab" },
          ]}
          onChange={shell.actions.setOpenFilesIn}
        />
      </div>
      <div className="check" data-setting="open-html-markdown-as">
        <span>
          Open HTML and Markdown as
          <span className="d">How these files open in Separate tabs.</span>
        </span>
        <Selector
          testId="settings.appearance-open-html-markdown-as"
          label="Open HTML and Markdown as"
          value={openAs}
          options={[
            { value: "preview", label: "Preview (default)" },
            { value: "code", label: "Code" },
            { value: "side", label: "Code with preview beside" },
          ]}
          onChange={shell.actions.setOpenHtmlMarkdownAs}
        />
      </div>
      <div className="check" data-setting="mark-vibe-lines">
        <span>
          Mark lines Vibe changes in open files
          <span className="d">
            On: the editor marks the lines Vibe changed while the file was open, until you press Got
            it.
          </span>
        </span>
        <Switch
          data-testid="settings.appearance-mark-vibe-lines"
          aria-label="Mark lines Vibe changes in open files"
          aria-checked={markVibeLines}
          onClick={() => {
            shell.actions.setMarkVibeLines(!markVibeLines);
          }}
        />
      </div>
      <h4>Transcript</h4>
      <div className="check" data-setting="show-thinking-expanded">
        <span>
          Show thinking expanded
          <span className="d">
            Open thinking in the session by default. You can still close each block yourself.
          </span>
        </span>
        <Switch
          data-testid="settings.appearance-settings-set-show-thinking-expanded"
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
