/**
 * Effects that connect the shell to the outside: the theme attribute on <html>, the single
 * keyboard listener, palette commands and shortcuts from `shellActions`, the `ui/*` routes (shell,
 * sessions, transcript, composer) and the loop that serves them. One hook so `Shell` stays a layout component.
 * Depends on: AppContext, actions, shellUiControl, state/uiControl, hooks. Called by: Shell.
 * Invariants: every registration is undone on unmount (React StrictMode mounts effects twice);
 * the UI-control loop only polls while the Debug API is on, or always in browser dev mode where
 * the Debug API is the transport itself.
 */
import { useEffect } from "react";

import { runUiControl } from "../../state/uiControl";
import { useApp } from "./AppContext";
import { shellActions } from "./actions";
import { useSessionWiring } from "../sessions/useSessionWiring";
import { useShellState } from "./hooks";
import { registerShellRoutes } from "./shellUiControl";
import { registerInputRoutes } from "./inputRoutes";
import { registerInspectRoutes } from "./inspectRoutes";
import { registerTranscriptRoutes } from "../transcript/transcriptRoutes";
import { registerComposerRoutes } from "../composer/composerRoutes";
import { registerDocumentRoutes } from "../composer/documentRoutes";
import { registerVoiceKeys } from "../voice/hooks";
import { registerVoiceRoutes } from "../voice/voiceRoutes";
import { registerSetupRoutes } from "../setup/setupRoutes";
import { registerSettingsRoutes } from "../settings/settingsRoutes";
import { registerConnectorRoutes } from "../connectors/connectorRoutes";
import { registerMarketRoutes } from "../marketplace/marketRoutes";
import { registerLibrariesRoutes } from "../libraries/librariesRoutes";
import { registerWorktreeRoutes } from "../worktrees/worktreeRoutes";
import { registerUpdatesRoutes } from "../updates/updatesRoutes";
import { registerProcessRoutes } from "./processRoutes";
import { registerSearchRoutes } from "../search/searchRoutes";
import { keyReadiness, vaultReadiness } from "../../state/keyReadiness";

/** Installs the shell's wiring for the lifetime of the component. */
export function useShellWiring(): void {
  const app = useApp();
  const theme = useShellState((s) => s.theme);

  useEffect(() => {
    document.documentElement.dataset["theme"] = theme;
  }, [theme]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // Dismiss the frontmost notification popover before Escape reaches background navigation.
      if (event.key === "Escape" && app.toasts.panel.get().open) {
        event.preventDefault();
        event.stopImmediatePropagation();
        app.toasts.close();
        return;
      }
      if (app.shortcuts.handle(event)) event.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
    };
  }, [app]);

  useEffect(() => {
    const removers = shellActions(app).flatMap((action) => [
      app.commands.register({
        id: action.id,
        label: action.label,
        section: "Actions",
        ...(action.keys === undefined ? {} : { keys: action.keys }),
        ...(action.keywords === undefined ? {} : { keywords: action.keywords }),
        run: action.run,
      }),
      ...(action.keys === undefined
        ? []
        : [
            app.shortcuts.register({
              id: action.id,
              keys: action.keys,
              description: action.label,
              run: action.run,
            }),
          ]),
    ]);
    return () => {
      for (const remove of removers) remove();
    };
  }, [app]);

  useEffect(() => registerShellRoutes(app), [app]);
  useEffect(() => registerInspectRoutes(app), [app]);
  useEffect(() => registerInputRoutes(app), [app]);
  useEffect(() => registerTranscriptRoutes(app), [app]);
  useEffect(() => registerComposerRoutes(app), [app]);
  useEffect(() => registerDocumentRoutes(app), [app]);
  useEffect(() => registerVoiceRoutes(app), [app]);
  useEffect(() => registerVoiceKeys(app), [app]);
  useEffect(() => registerSetupRoutes(app), [app]);
  useEffect(() => registerSettingsRoutes(app), [app]);
  useEffect(() => registerConnectorRoutes(app), [app]);
  useEffect(() => registerMarketRoutes(app), [app]);
  useEffect(() => registerLibrariesRoutes(app), [app]);
  useEffect(() => registerWorktreeRoutes(app), [app]);
  useEffect(() => app.worktrees?.watch(), [app]);
  useEffect(() => registerUpdatesRoutes(app), [app]);
  useEffect(() => {
    void app.updates?.start();
  }, [app]);
  // processes deleted sessions left running (status bar list, `ui/processes`).
  useEffect(() => app.leftRunning?.start(), [app]);
  // the open session's processes (the button's count; live output while the dialog shows).
  useEffect(
    () => app.sessionProcesses?.start(() => app.leftRunning?.store.get().open === true),
    [app],
  );
  useEffect(() => registerProcessRoutes(app), [app]);
  useEffect(() => registerSearchRoutes(app), [app]);
  // the temporary sessions (badge, banner) and their pruning when sessions go away.
  useEffect(() => {
    const tools = app.sessionTools;
    const core = app.core;
    if (tools === null || core === null) return;
    const stop = tools.start();
    const unhook = core.onSessionsGone((ids) => {
      tools.forget(ids);
    });
    return () => {
      stop();
      unhook();
    };
  }, [app]);
  useSessionWiring();

  // The Mistral key controller: reads the vault status once the window is up.
  useEffect(() => {
    void app.keys?.start();
    // whether Vibe is installed (the setup screen offers the install).
    void app.vibe?.start();
  }, [app]);

  // Catalogs suppressed while the key is unavailable refresh in place after setup or unlock.
  useEffect(() => {
    const keys = app.keys;
    if (keys === null) return;
    let blocked = keyReadiness(keys.store.get()) !== null;
    let vaultBlocked = vaultReadiness(keys.store.get()) !== null;
    return keys.store.subscribe(() => {
      const next = keyReadiness(keys.store.get()) !== null;
      const nextVault = vaultReadiness(keys.store.get()) !== null;
      if ((blocked && !next) || (vaultBlocked && !nextVault))
        void app.connectors?.onKeyReadinessChanged();
      blocked = next;
      vaultBlocked = nextVault;
    });
  }, [app]);

  useEffect(() => {
    const core = app.core;
    if (core === null) return;
    const abort = new AbortController();
    const active = () => core.client.kind !== "tauri" || core.store.get().debugApi.enabled;
    void runUiControl(core.client.uiControl, app.uiControl, active, abort.signal);
    return () => {
      abort.abort();
    };
  }, [app]);
}
