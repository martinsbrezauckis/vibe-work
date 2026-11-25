/**
 * Global dictation shortcut (**Mod+Shift+Space**, hold-to-talk >= 400 ms or short press toggle)
 * and Escape cancellation, decoupled from `features/voice` UI/runtime so shell wiring stays eager-light.
 * Depends on: keyReadiness, AppContext types. Called by: useShellWiring, re-exported by features/voice/hooks.
 */
import type { App } from "../features/shell/AppContext";
import { keyReadiness } from "./keyReadiness";

/** A press shorter than this toggles; a longer one is hold-to-talk. */
export const HOLD_MS = 400;

/** The voice shortcut. */
export const DICTATE_KEYS = "Mod+Shift+space";

/**
 * Registers the dictation shortcut and Escape (cancel). A press while the previous text is still
 * being made is queued (`dictation.queueStart`).
 */
export function registerVoiceKeys(app: App): () => void {
  const voice = app.voice;
  if (voice === null) return () => undefined;
  const { dictation } = voice;
  let down: { at: number; started: boolean } | null = null;
  const removeShortcut = app.shortcuts.register({
    id: "voice.dictate",
    keys: DICTATE_KEYS,
    description: "Dictate into the message box (hold to talk, or press to start and stop)",
    allowInInput: true,
    run: () => {
      if (app.keys && keyReadiness(app.keys.store.get()) !== null) return;
      if (down !== null) return; // a key repeat while held
      const phase = dictation.store.get().phase;
      const session = app.shell.store.get().selected;
      if ((phase === "idle" || phase === "error") && session !== null) {
        down = { at: performance.now(), started: true };
        void dictation.start(session);
      } else if (phase === "transcribing") {
        // The previous text is still being made: this press is queued (a second one cancels it).
        down = { at: performance.now(), started: false };
        if (session !== null) dictation.queueStart(session);
      } else {
        down = { at: performance.now(), started: false };
        if (phase === "recording" || phase === "starting") void dictation.stop();
      }
    },
  });
  const onKeyUp = (e: KeyboardEvent) => {
    if (e.code !== "Space" || down === null) return;
    const held = performance.now() - down.at;
    const started = down.started;
    down = null;
    if (started && held >= HOLD_MS) void dictation.stop();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    const phase = dictation.store.get().phase;
    if (phase === "recording" || phase === "starting") {
      e.preventDefault();
      dictation.cancel();
    }
  };
  window.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("keydown", onKeyDown, true);
  return () => {
    removeShortcut();
    window.removeEventListener("keyup", onKeyUp, true);
    window.removeEventListener("keydown", onKeyDown, true);
  };
}
