/**
 * The shell's own actions, defined once and offered twice: as palette commands and as keyboard
 * shortcuts (new session, the waiting prompt, list toggle, tabs, theme and settings).
 * Depends on: AppContext types, state/panelLayout, state/shellState. Called by: useShellWiring.
 * Invariants: an action reads the current state when it runs (never a stale closure); the same
 * combo string drives the shortcut and the label shown in the palette.
 */
import { TEXT } from "../../vocabulary";
import { computeLayout } from "../../state/panelLayout";
import { TAB_META } from "../../state/shellState";
import { switchTheme } from "../../state/themeSwitch";
import type { App } from "./AppContext";
import { openNewChat } from "./newChat";
import { GO_TO_PROMPT_KEYS, goToWaitingPrompt } from "../ask/waiting";

/** One shell action. */
export interface ShellAction {
  id: string;
  label: string;
  /** Combo (`Mod+K` style) for both the shortcut and the palette label; omit for palette-only. */
  keys?: string;
  /** Extra words the palette matches (for example an older name). */
  keywords?: string;
  run: () => void;
}

/** The actions for `app`. */
export function shellActions(app: App): ShellAction[] {
  const { shell } = app;
  const a = shell.actions;
  const layout = () => computeLayout(shell.store.get(), window.innerWidth);
  return [
    {
      id: "palette",
      label: "Open the command palette",
      keys: "Mod+K",
      run: () => {
        a.setOverlay("palette");
      },
    },
    {
      // one New chat everywhere (rail, this shortcut, the list's button): the new-chat view, composer
      // focused; the session starts on the first send. The id stays (palettes and tests know it).
      id: "new-session",
      label: TEXT.newChat,
      keys: "Mod+N",
      // Its name before: someone typing "new session" still finds it.
      keywords: "new session start conversation",
      run: () => {
        openNewChat(app);
      },
    },
    {
      id: "toggle-list",
      label: "Hide or show the list",
      keys: "Mod+B",
      run: () => {
        const l = layout();
        if (l.drawer) a.toggleDrawer();
        else a.toggleList(l.listVisible);
      },
    },
    {
      id: "open-project",
      label: "Open the Project tab",
      run: () => {
        a.openTab("project");
      },
    },
    {
      id: "open-browser",
      label: `Open the ${TAB_META.browser.label} tab`,
      run: () => {
        a.openTab("browser");
      },
    },
    {
      id: "open-files",
      label: `Open the ${TAB_META.git.label} tab`,
      run: () => {
        a.openTab("git");
      },
    },
    {
      id: "open-changes",
      label: `Open the ${TAB_META.changes.label} tab`,
      run: () => {
        a.openTab("changes");
      },
    },
    {
      id: "open-terminal",
      label: `Open the ${TAB_META.terminal.label} tab`,
      keys: "Mod+`",
      run: () => {
        a.openTab("terminal");
      },
    },
    { id: "close-panel", label: "Close the side panel", run: a.closePanel },
    {
      id: "open-skills",
      label: "Open Skills",
      run: () => {
        void app.market?.actions.showTab("skills");
      },
    },
    {
      id: "open-connectors",
      label: "Open connectors and MCP servers",
      run: () => {
        void app.connectors?.actions.open();
      },
    },
    {
      // the open session as Markdown, through the native save dialog.
      id: "export-session",
      label: "Export this session as Markdown",
      keywords: "save download markdown md transcript",
      run: () => {
        const id = shell.store.get().selected;
        if (id !== null) void app.sessionTools?.exportSession(id);
      },
    },
    {
      id: "toggle-theme",
      label: "Toggle theme",
      run: () => {
        switchTheme(shell.store.get().theme === "dark" ? "light" : "dark", a.setTheme);
      },
    },
    {
      id: "open-settings",
      label: "Open settings",
      keys: "Mod+,",
      run: () => {
        a.openSettings();
      },
    },
    {
      // The way back to a card for someone who cannot see where it is.
      id: "go-to-prompt",
      label: TEXT.askGoTo,
      keys: GO_TO_PROMPT_KEYS,
      keywords: "permission prompt approval card answer waiting ask question allow",
      run: () => {
        goToWaitingPrompt(app);
      },
    },
  ];
}
