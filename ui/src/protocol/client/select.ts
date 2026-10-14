/**
 * Picks the client for this run: Tauri IPC inside the app, the Debug API dev proxy in a browser
 * development build, otherwise nothing (a production bundle opened in a plain browser has no
 * core to talk to and says so).
 * Depends on: ./tauriClient, ./debugApiClient. Called by: main.tsx.
 */
import { createDebugApiClient } from "./debugApiClient";
import { createTauriClient, insideTauri } from "./tauriClient";
import type { CoreClient } from "./types";

/** The client for this environment, or `null` when there is no way to reach a core. */
export function selectClient(): CoreClient | null {
  if (insideTauri()) return createTauriClient();
  if (import.meta.env.DEV) return createDebugApiClient();
  return null;
}
