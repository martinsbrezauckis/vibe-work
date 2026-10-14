/**
 * The shell's dependency container: the stores and registries one running app is made of, handed
 * to components through React context so tests can build a whole shell from fakes.
 * Depends on: state/*. Called by: main.tsx (provides), every shell component (`useApp`).
 */
import { type ReactNode, createContext, useContext } from "react";

import type { CoreController } from "../../state/coreStore";
import type { Commands } from "../../state/commands";
import type { Composer } from "../../state/composer";
import type { Projects } from "../../state/projects";
import type { Connectors } from "../../state/connectors";
import type { Market } from "../../state/market";
import type { Documents } from "../../state/documents";
import type { Libraries } from "../../state/libraries";
import type { Files } from "../../state/files";
import type { Keys } from "../../state/keyStore";
import type { Notifier } from "../../state/notifyStore";
import type { SessionActions } from "../../state/sessionActions";
import type { PanelTabs } from "../../state/panelTabs";
import type { Shell } from "../../state/shellStore";
import type { Shortcuts } from "../../state/shortcuts";
import type { Toasts } from "../../state/toastStore";
import type { Transcript } from "../../state/transcriptStore";
import type { Updates } from "../../state/updates";
import type { VibeInstall } from "../../state/vibeInstall";
import type { UiControl } from "../../state/uiControl";
import type { Voice } from "../../state/voice";
import type { Worktrees } from "../../state/worktrees";
import type { Git } from "../../state/git";
import type { LeftRunning } from "../../state/leftRunning";
import type { SessionTools } from "../../state/sessionTools";
import type { Find } from "../../state/findStore";
import type { SessionProcesses } from "../../state/sessionProcesses";

/** Everything one app instance holds. */
export interface App {
  core: CoreController | null;
  /** Row and folder actions; `null` without a core. */
  sessions: SessionActions | null;
  /** The transcript's UI state and actions (open groups, callback answers, links). */
  transcript: Transcript;
  /** The composer: drafts, chips, images and the send / session-control actions. */
  composer: Composer;
  /** The Projects overview; `null` without a core. */
  projects: Projects | null;
  /** Documents in the composers (picker, drop, OCR); `null` without a core. */
  documents: Documents | null;
  /** Files the agent mentions: clickable references and the File tab; `null` without a core. */
  files: Files | null;
  /** Dictation, narration and their settings; `null` without a core. */
  voice: Voice | null;
  /** The Connectors page; `null` without a core. */
  connectors: Connectors | null;
  /** The Tools page's Marketplace tab; `null` without a core. */
  market: Market | null;
  /** The Libraries page; `null` without a core. */
  libraries: Libraries | null;
  /** Worktree per session: copies, end-of-session choice, orphans; `null` without a core. */
  worktrees: Worktrees | null;
  /** The outer git loop of the Changes tab: branch bar, Commit, Push, PR, PR card; `null` without a core. */
  git: Git | null;
  /** Updates: the host's update flow as the window shows it; `null` without a core. */
  updates: Updates | null;
  /** Processes deleted sessions left running: the after-delete question and the list; `null` without a core. */
  leftRunning: LeftRunning | null;
  /** Export as Markdown and temporary sessions; `null` without a core. */
  sessionTools: SessionTools | null;
  /** The open session's dev servers and long commands (the Processes panel); `null` without a core. */
  sessionProcesses: SessionProcesses | null;
  /** Find in this conversation and in the shown file. */
  find: Find;
  /** The Mistral key and vault controller (setup, Settings, unlock prompt); `null` without a core. */
  keys: Keys | null;
  /** The Vibe install at setup; `null` without a core. */
  vibe: VibeInstall | null;
  /** OS notification settings and watcher. */
  notifier: Notifier;
  shell: Shell;
  toasts: Toasts;
  commands: Commands;
  shortcuts: Shortcuts;
  uiControl: UiControl;
  panelTabs: PanelTabs;
}

const Context = createContext<App | null>(null);

/** Provides `app` to the tree. */
export function AppProvider({ app, children }: { app: App; children: ReactNode }) {
  return <Context.Provider value={app}>{children}</Context.Provider>;
}

/** The app instance; throws when used outside `AppProvider` (a wiring bug, not a runtime state). */
export function useApp(): App {
  const app = useContext(Context);
  if (app === null) throw new Error("useApp outside AppProvider");
  return app;
}
