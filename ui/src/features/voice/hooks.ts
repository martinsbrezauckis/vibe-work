/**
 * Voice hooks: read the dictation, narration and voice chat stores, keep the text box's caret known to
 * dictation (so text lands where the user types), and the keyboard: **Mod+Shift+Space** (hold to talk:
 * released after 400 ms stops; a short press toggles), Escape cancels a running dictation.
 * Depends on: AppContext, createStore, state/dictation, state/narration, shortcuts registry. Called by:
 * Composer, MicButton, DictationStrip, NarrationBar, useShellWiring.
 * Invariants: selectors return stable values; listeners are removed on unmount; key repeats of a held
 * shortcut never toggle twice.
 */
import { type RefObject, useEffect } from "react";

import { createStore, useStore } from "../../state/createStore";
import { type CaretPlacement, DICTATION_IDLE, type DictationState } from "../../state/dictation";
import { NARRATION_IDLE, type NarrationState } from "../../state/narration";
import { VOICE_CHAT_OFF, type VoiceChatState } from "../../state/voiceChat";
import { VOICE_DEFAULTS, type VoiceSettings } from "../../state/voiceSettings";
import { useApp } from "../shell/AppContext";
import { keyReadiness } from "../../state/keyReadiness";
import type { App } from "../shell/AppContext";

/** A press shorter than this toggles; a longer one is hold-to-talk. */
export const HOLD_MS = 400;

/** The voice shortcut. */
export const DICTATE_KEYS = "Mod+Shift+space";

// Stand-ins when the app has no core (no voice): they never change.
const NO_DICTATION = createStore<DictationState>(DICTATION_IDLE);
const NO_NARRATION = createStore<NarrationState>(NARRATION_IDLE);
const NO_SETTINGS = createStore<VoiceSettings>(VOICE_DEFAULTS);
const NO_PLACED = createStore<CaretPlacement>(null);
const NO_VOICE_CHAT = createStore<VoiceChatState>(VOICE_CHAT_OFF);

/** Selects from the dictation state (idle without voice). */
export function useDictation<S>(select: (s: DictationState) => S): S {
  return useStore(useApp().voice?.dictation.store ?? NO_DICTATION, select);
}

/** Selects from the narration state (idle without voice). */
export function useNarration<S>(select: (s: NarrationState) => S): S {
  return useStore(useApp().voice?.narration.store ?? NO_NARRATION, select);
}

/** Selects from the voice chat state (off without voice). */
export function useVoiceChat<S>(select: (s: VoiceChatState) => S): S {
  return useStore(useApp().voice?.voiceChat.store ?? NO_VOICE_CHAT, select);
}

/** Selects from the voice settings (the defaults without voice). */
export function useVoiceSettings<S>(select: (s: VoiceSettings) => S): S {
  return useStore(useApp().voice?.settings.store ?? NO_SETTINGS, select);
}

/**
 * Wires a text box to dictation: lets dictation read the box's own selection when it inserts text (the
 * events below are only the fallback), reports the selection on every change, and moves the caret after
 * the inserted text when dictation put some in.
 */
export function useDictationCaret(sessionId: string, box: RefObject<HTMLTextAreaElement | null>) {
  const voice = useApp().voice;
  const placed = useStore(voice?.dictation.placed ?? NO_PLACED, (p) => p);
  useEffect(() => {
    if (placed === null || placed.sessionId !== sessionId) return;
    const el = box.current;
    if (el === null) return;
    el.setSelectionRange(placed.caret, placed.caret);
  }, [placed, sessionId, box]);
  useEffect(
    () =>
      voice?.dictation.bindCaretReader(sessionId, (draft) => {
        const el = box.current;
        // Only the selection of a mounted box that shows the text being edited means something.
        if (el === null || !el.isConnected || el.value !== draft) return null;
        return { start: el.selectionStart, end: el.selectionEnd };
      }),
    [voice, sessionId, box],
  );
  return () => {
    const el = box.current;
    if (el !== null) voice?.dictation.setCaret(sessionId, el.selectionStart, el.selectionEnd);
  };
}

/** Registers the dictation shortcut and Escape (cancel). A press while the previous text is still being made is queued
 * (`dictation.queueStart`). Called once by the shell wiring. */
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
        down = { at: Date.now(), started: true };
        void dictation.start(session);
      } else if (phase === "transcribing") {
        // The previous text is still being made: this press is queued (a second one cancels it),.
        down = { at: Date.now(), started: false };
        if (session !== null) dictation.queueStart(session);
      } else {
        down = { at: Date.now(), started: false };
        if (phase === "recording" || phase === "starting") void dictation.stop();
      }
    },
  });
  const onKeyUp = (e: KeyboardEvent) => {
    if (e.code !== "Space" || down === null) return;
    const held = Date.now() - down.at;
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

/** Elapsed whole seconds as `m:ss`. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(s / 60))}:${String(s % 60).padStart(2, "0")}`;
}
