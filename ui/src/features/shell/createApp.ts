/**
 * Builds one app instance (the stores, registries and controllers a running window is made of) from
 * a core client. `main.tsx` and the test rig both use it, so the wiring they exercise is the
 * same.
 * Depends on: state/*. Called by: main.tsx, the UI tests.
 * Invariants: no side effects beyond creating objects (starting the core, registering listeners and
 * mounting happen in `main.tsx` / `useShellWiring`); a missing client gives an app without a core
 * and without session actions, which renders the "no connection" state.
 */
import { createSecretScan } from "../../state/secretScan";
import { createVibeInstall } from "../../state/vibeInstall";
import type { CoreClient } from "../../protocol/client/types";
import { createCommands } from "../../state/commands";
import { createComposer } from "../../state/composer";
import { createConnectors } from "../../state/connectors";
import { createMarket } from "../../state/market";
import { createCore } from "../../state/coreStore";
import { createDocuments } from "../../state/documents";
import { createFiles } from "../../state/files";
import { createKeys } from "../../state/keyStore";
import { keyReadiness, vaultReadiness } from "../../state/keyReadiness";
import { type Libraries, createLibraries } from "../../state/libraries";
import { createLeftRunning } from "../../state/leftRunning";
import { createProjects } from "../../state/projects";
import { createNotifier } from "../../state/notifyStore";
import { createPanelTabs } from "../../state/panelTabs";
import { createSessionActions } from "../../state/sessionActions";
import { createSessionTools } from "../../state/sessionTools";
import { createFind } from "../../state/findStore";
import { createSessionProcesses } from "../../state/sessionProcesses";
import { type Shell, createShell } from "../../state/shellStore";
import { type Shortcuts, createShortcuts } from "../../state/shortcuts";
import { createToasts } from "../../state/toastStore";
import { createTranscript } from "../../state/transcriptStore";
import { createUiControl } from "../../state/uiControl";
import { createUpdates } from "../../state/updates";
import { createVoice } from "../../state/voice";
import { createWorktrees } from "../../state/worktrees";
import { createGit } from "../../state/git";
import type { PrepareCapture } from "../voice/pcmCapture";
import type { Player } from "../voice/audioPlayer";
import type { App } from "./AppContext";

/** Options for tests; the app passes only `client`. */
export interface CreateAppOptions {
  client: CoreClient | null;
  /** Use localStorage for the theme, widths and notification level (default true). */
  persist?: boolean;
  shell?: Shell;
  shortcuts?: Shortcuts;
  /** Test fakes for the microphone and the speaker. */
  prepareCapture?: PrepareCapture;
  player?: Player;
}

/**
 * the Libraries page lies over the work view like the Projects overview and the Tools page. Opening it
 * closes them and Settings; opening one of them (or Settings) closes it. Wired here once, so the older pages' many
 * open paths (rail, palette, routes) need no change.
 */
function pagesExclusive(
  shell: Shell,
  projects: App["projects"],
  connectors: App["connectors"],
  libraries: Libraries | null,
): void {
  if (libraries === null) return;
  let was = false;
  libraries.store.subscribe(() => {
    const open = libraries.store.get().open;
    if (open && !was) {
      shell.actions.closeSettings();
      projects?.close();
      connectors?.actions.close();
    }
    was = open;
  });
  const closeLibraries = () => {
    if (libraries.store.get().open) libraries.actions.close();
  };
  projects?.store.subscribe(() => {
    if (projects.store.get().open) closeLibraries();
  });
  connectors?.store.subscribe(() => {
    if (connectors.store.get().open) closeLibraries();
  });
  shell.store.subscribe(() => {
    if (shell.store.get().settings.open) closeLibraries();
  });
}

/** Creates the app. */
export function createApp(options: CreateAppOptions): App {
  const toasts = createToasts();
  const uiControl = createUiControl();
  const shell = options.shell ?? createShell({ persist: options.persist ?? true });
  const core =
    options.client === null
      ? null
      : createCore(
          options.client,
          (toast) => toasts.push(toast),
          () => uiControl.servingCapability(),
          (capability, promise) => uiControl.track(capability, promise),
          (command) => uiControl.trace.record(command, uiControl.trigger()),
          (capability, promise) => uiControl.track(capability, promise, true),
        );
  const worktrees =
    core === null
      ? null
      : createWorktrees({
          core,
          shell,
          toasts,
          persist: options.persist ?? true,
          runCommand: (id, name, args, origin): Promise<unknown> => {
            const actions = origin === "debug-api" ? composer.debugApi : composer.actions;
            if (actions === null) throw new Error("The composer is unavailable.");
            return actions.runCommand(id, name, args);
          },
        });
  const leftRunning = core === null ? null : createLeftRunning({ core, toasts });
  const sessions =
    core === null
      ? null
      : createSessionActions({
          core,
          shell,
          toasts,
          afterDelete: (id) => leftRunning?.afterDelete(id) ?? Promise.resolve(null),
          ended: (id, after) => worktrees?.ended(id, after) ?? Promise.resolve(),
        });
  const keys = core === null ? null : createKeys(core);
  const keyGuard = () => (keys === null ? null : (keyReadiness(keys.store.get())?.message ?? null));
  const canUseKey = () => keyGuard() === null;
  const canUseVault = () => keys === null || vaultReadiness(keys.store.get()) === null;
  const composer = createComposer({
    core,
    shell,
    toasts,
    sessions,
    persist: options.persist ?? true,
    keyGuard,
    // `/mcp` and `/connectors` do what the rail's plug button does. Called only after the app is built,
    // so `connectors` and `projects` (made below) are set by then.
    openTools: () => {
      shell.actions.closeSettings();
      projects?.close();
      void connectors?.actions.open();
    },
  });
  const transcript = createTranscript({ core, toasts, persist: options.persist ?? true });
  // a prepared message ("Send failing checks to Vibe") becomes a context chip, never a send.
  const git =
    core === null
      ? null
      : createGit({
          core,
          insertChip: (sessionId, label, text, origin) => {
            const facade = origin === "debug-api" ? composer.debugApi : composer.actions;
            facade?.insertContext(sessionId, { label, source: "git", text });
          },
        });
  const connectors =
    core === null
      ? null
      : createConnectors({ core, shell, transcript, toasts, canUseKey, canUseVault });
  const projects =
    core === null ? null : createProjects({ core, shell, sessions, composer, toasts });
  const libraries =
    core === null || options.client === null
      ? null
      : createLibraries({ core, client: options.client, shell, keyReady: canUseKey });
  pagesExclusive(shell, projects, connectors, libraries);
  return {
    core,
    secretScan: core === null ? null : createSecretScan(core, toasts),
    sessions,
    keys,
    vibe: core === null ? null : createVibeInstall(core),
    updates:
      options.client === null
        ? null
        : createUpdates({
            client: options.client,
            toasts,
            serving: () => uiControl.servingCapability(),
          }),
    transcript,
    connectors,
    libraries,
    market:
      core === null || connectors === null
        ? null
        : createMarket({ core, shell, connectors, toasts, persist: options.persist ?? true }),
    worktrees,
    git,
    leftRunning,
    sessionTools: core === null ? null : createSessionTools({ core, shell, toasts }),
    find: createFind(),
    sessionProcesses: core === null ? null : createSessionProcesses({ core, shell, toasts }),
    composer,
    projects,
    documents: core === null ? null : createDocuments({ core, composer, shell, toasts, canUseKey }),
    files: core === null ? null : createFiles(core),
    voice:
      core === null
        ? null
        : createVoice({
            core,
            composer,
            shell,
            toasts,
            persist: options.persist ?? true,
            canUseKey,
            ...(options.prepareCapture === undefined ? {} : { prepare: options.prepareCapture }),
            ...(options.player === undefined ? {} : { player: options.player }),
          }),
    notifier: createNotifier({ persist: options.persist ?? true }),
    shell,
    toasts,
    commands: createCommands(),
    shortcuts: options.shortcuts ?? createShortcuts(),
    uiControl,
    panelTabs: createPanelTabs(),
  };
}
