/**
 * The "show the provider's own error text" preference: off by default, kept in
 * `localStorage` like the theme (a per-computer display choice, not Vibe config).
 * Depends on: createStore, providerError. Called by: features/settings/ErrorDetailSetting,
 * features/transcript/rows (through `useErrorText`).
 * Invariants: storage can be missing or blocked, so every access is wrapped and the default (plain)
 * applies; reads validate the stored value; writes never throw.
 */
import { createStore, useStore } from "./createStore";
import { type ErrorDetailMode, errorText } from "./providerError";

const KEY = "vw.errorDetail";

function load(): ErrorDetailMode {
  try {
    return localStorage.getItem(KEY) === "details" ? "details" : "plain";
  } catch {
    return "plain";
  }
}

/** The preference. */
export const errorDetail = createStore<ErrorDetailMode>(load());

/** Sets and remembers the preference. */
export function setErrorDetail(mode: ErrorDetailMode): void {
  errorDetail.set(mode);
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    // Not remembered; the choice still applies until the window closes.
  }
}

/** Vibe's error text as the preference says to show it. */
export function useErrorText(raw: string): string {
  const mode = useStore(errorDetail, (m) => m);
  return errorText(raw, mode);
}
