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
import { DICTATE_KEYS, HOLD_MS, registerVoiceKeys } from "../../state/voiceKeys";
import {
  type CaretPlacement,
  DICTATION_IDLE,
  type DictationState,
  NARRATION_IDLE,
  type NarrationState,
  VOICE_CHAT_OFF,
  VOICE_CHUNK_IDLE,
  type VoiceChatState,
  type VoiceChunkLoadState,
  type VoicePromptState,
  type VoiceQuestionState,
} from "../../state/voiceTypes";
import { VOICE_DEFAULTS, type VoiceSettings } from "../../state/voiceSettings";
import { useApp } from "../shell/AppContext";

export { DICTATE_KEYS, HOLD_MS, registerVoiceKeys };

// Stand-ins when the app has no core (no voice): they never change.
const NO_DICTATION = createStore<DictationState>(DICTATION_IDLE);
const NO_NARRATION = createStore<NarrationState>(NARRATION_IDLE);
const NO_SETTINGS = createStore<VoiceSettings>(VOICE_DEFAULTS);
const NO_PLACED = createStore<CaretPlacement>(null);
const NO_PROMPT = createStore<VoicePromptState | null>(null);
const NO_QUESTION = createStore<VoiceQuestionState | null>(null);
const NO_VOICE_CHAT = createStore<VoiceChatState>(VOICE_CHAT_OFF);
const NO_CHUNK_LOAD = createStore<VoiceChunkLoadState>(VOICE_CHUNK_IDLE);

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

/** The single card currently being read or answered by voice. */
export function useVoicePrompt<S>(select: (s: VoicePromptState | null) => S): S {
  return useStore(useApp().voice?.voiceChat.prompt.store ?? NO_PROMPT, select);
}

/** The question card being answered by voice in voice chat, or `null`. */
export function useVoiceQuestion<S>(select: (s: VoiceQuestionState | null) => S): S {
  return useStore(useApp().voice?.voiceChat.question.store ?? NO_QUESTION, select);
}

/** Selects from the voice settings (the defaults without voice). */
export function useVoiceSettings<S>(select: (s: VoiceSettings) => S): S {
  return useStore(useApp().voice?.settings.store ?? NO_SETTINGS, select);
}

/** Selects the status of loading the on-demand voice runtime chunk. */
export function useVoiceChunkLoad<S>(select: (s: VoiceChunkLoadState) => S): S {
  return useStore(useApp().voice?.chunkLoad.store ?? NO_CHUNK_LOAD, select);
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

/** Elapsed whole seconds as `m:ss`. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(s / 60))}:${String(s % 60).padStart(2, "0")}`;
}
