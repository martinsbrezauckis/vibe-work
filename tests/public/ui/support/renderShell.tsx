/**
 * Builds a whole app instance from fakes (the same wiring as ui/src/main.tsx) and renders the
 * shell into jsdom at a chosen window width. Synthetic data only.
 */
import type { Player } from "../../../../ui/src/features/voice/audioPlayer";
import type { PrepareCapture } from "../../../../ui/src/features/voice/pcmCapture";
import { act, render } from "@testing-library/react";

import {
  type App,
  AppProvider,
} from "../../../../ui/src/features/shell/AppContext";
import { createApp } from "../../../../ui/src/features/shell/createApp";
import { registerPlaceholderTabs } from "../../../../ui/src/features/shell/PlaceholderTabs";
import { Shell } from "../../../../ui/src/features/shell/Shell";
import { registerTerminalTab } from "../../../../ui/src/features/terminal/register";
import { createShell } from "../../../../ui/src/state/shellStore";
import { createShortcuts } from "../../../../ui/src/state/shortcuts";
import { type FakeClient, fakeClient } from "./fakeClient";
import { composerBox, preloadLazy } from "./lazyReady";

export function setWidth(width: number): void {
  Object.defineProperty(window, "innerWidth", {
    value: width,
    configurable: true,
    writable: true,
  });
  Object.defineProperty(window, "innerHeight", {
    value: 800,
    configurable: true,
    writable: true,
  });
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });
}

export interface Rig {
  app: App;
  client: FakeClient;
  view: ReturnType<typeof render>;
  /**
   * Resolves with the composer textarea once the lazily loaded composer is mounted (a session must be
   * selected). Use this instead of a synchronous `getByRole("textbox", { name: "Message" })`.
   */
  composer(): Promise<HTMLTextAreaElement>;
}

export async function renderShell(
  options: {
    width?: number;
    kind?: FakeClient["kind"];
    persist?: boolean;
    setup?: (client: FakeClient) => void;
    /** Registers the terminal tab and route (as main.tsx does). */
    terminal?: boolean;
    /** Fake microphone and speaker. */
    prepareCapture?: PrepareCapture;
    /** Handle Mod as Cmd (a Mac); default Ctrl. */
    mac?: boolean;
    player?: Player;
  } = {},
): Promise<Rig> {
  // Lazy chunks first: the modules are cached before anything renders (see lazyReady.ts).
  await preloadLazy({ terminal: options.terminal === true });
  setWidth(options.width ?? 1400);
  const client = fakeClient(options.kind);
  options.setup?.(client);
  const app = createApp({
    client,
    persist: options.persist ?? false,
    shell: createShell({ persist: options.persist ?? false, theme: "dark" }),
    // Mac key handling only when a test asks for it.
    shortcuts: createShortcuts(options.mac ?? false),
    ...(options.prepareCapture === undefined
      ? {}
      : { prepareCapture: options.prepareCapture }),
    ...(options.player === undefined ? {} : { player: options.player }),
  });
  // The host attributes what the window sends while it serves a Debug API action to the Debug API (its driving
  // rule); tests that call `uiControl.handle` directly have no transport, so the fake asks the page.
  client.servingProbe = () => app.uiControl.servingAction() !== null;
  registerPlaceholderTabs(app.panelTabs);
  if (options.terminal === true) registerTerminalTab(app);
  const view = render(
    <AppProvider app={app}>
      <Shell />
    </AppProvider>,
  );
  await act(async () => {
    await app.core?.start();
    await app.newChat?.start();
  });
  return { app, client, view, composer: composerBox };
}
