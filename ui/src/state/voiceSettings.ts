/**
 * Voice settings: microphone, dictation language, narration on/off and its voice, send after
 * dictation, and voice chat's "End my turn after a pause" (on by default) and pause length
 * (0.8-2.5 s, default 0.8 s). Kept in
 * `localStorage` like the theme: per-computer choices, never Vibe config.
 * Depends on: createStore. Called by: features/settings/VoiceSettings, state/dictation, state/narration.
 * Invariants: **narration is off by default**; reads validate the stored value (a stale
 * or tampered value falls back to the default); storage can be missing or blocked, so every access is
 * wrapped and writes never throw.
 */
import { PAUSE_DEFAULT_MS, PAUSE_MAX_MS, PAUSE_MIN_MS } from "../features/voice/turnEnd";
import { createStore } from "./createStore";

const KEY = "vw.voice";

/** Dictation languages a user can fix (the 13 Voxtral transcribes; `crates/vw-core-api` `LANGUAGES`). */
export const DICTATION_LANGUAGES: readonly { code: string; label: string }[] = [
  { code: "en", label: "English" },
  { code: "fr", label: "French" },
  { code: "de", label: "German" },
  { code: "es", label: "Spanish" },
  { code: "it", label: "Italian" },
  { code: "pt", label: "Portuguese" },
  { code: "nl", label: "Dutch" },
  { code: "ru", label: "Russian" },
  { code: "ar", label: "Arabic" },
  { code: "hi", label: "Hindi" },
  { code: "zh", label: "Chinese" },
  { code: "ja", label: "Japanese" },
  { code: "ko", label: "Korean" },
];

/** The settings. */
export interface VoiceSettings {
  /** The microphone's `deviceId`, or `null` for the system default. */
  micId: string | null;
  /** `null` = Auto (live text); a code from `DICTATION_LANGUAGES` = text on release. */
  language: string | null;
  /** Read finished turns aloud (off by default). */
  narration: boolean;
  /** Narration voice id (`null` = Mistral's default for Vibe, `gb_jane_neutral`); voice chat speaks with it too. */
  voice: string | null;
  /** Send the message as soon as the dictated text is in (off by default). */
  autosend: boolean;
  /** Voice chat: end the user's turn after a pause (on by default). */
  voiceChatAutoEnd: boolean;
  /** Voice chat: the pause that ends the turn, in ms (`PAUSE_MIN_MS`..`PAUSE_MAX_MS`). */
  voiceChatPauseMs: number;
}

/** The defaults: system microphone, Auto, narration off, default voice. */
export const VOICE_DEFAULTS: VoiceSettings = {
  micId: null,
  language: null,
  narration: false,
  voice: null,
  autosend: false,
  voiceChatAutoEnd: true,
  voiceChatPauseMs: PAUSE_DEFAULT_MS,
};

const ID = /^[A-Za-z0-9_+/=.-]{1,200}$/;

/** A pause length in ms within the slider's range (100 ms steps); anything else is the default. */
function pauseMs(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return PAUSE_DEFAULT_MS;
  return Math.round(Math.min(PAUSE_MAX_MS, Math.max(PAUSE_MIN_MS, value)) / 100) * 100;
}

/** Validates a stored value; anything unexpected becomes the default for that field. */
export function parseVoiceSettings(raw: unknown): VoiceSettings {
  if (typeof raw !== "object" || raw === null) return VOICE_DEFAULTS;
  const r = raw as Record<string, unknown>;
  const str = (v: unknown) => (typeof v === "string" && ID.test(v) ? v : null);
  const language = str(r["language"]);
  return {
    micId: str(r["micId"]),
    language: DICTATION_LANGUAGES.some((l) => l.code === language) ? language : null,
    narration: r["narration"] === true,
    voice: str(r["voice"]),
    autosend: r["autosend"] === true,
    voiceChatAutoEnd: r["voiceChatAutoEnd"] !== false,
    voiceChatPauseMs: pauseMs(r["voiceChatPauseMs"]),
  };
}

function load(persist: boolean): VoiceSettings {
  if (!persist) return VOICE_DEFAULTS;
  try {
    const stored = localStorage.getItem(KEY);
    return stored === null ? VOICE_DEFAULTS : parseVoiceSettings(JSON.parse(stored));
  } catch {
    return VOICE_DEFAULTS;
  }
}

/** Creates the settings store; `persist` reads and writes `localStorage`. */
export function createVoiceSettings(persist: boolean) {
  const store = createStore<VoiceSettings>(load(persist));
  return {
    store,
    /** Changes some fields and remembers them. */
    update(patch: Partial<VoiceSettings>): void {
      store.set((s) => parseVoiceSettings({ ...s, ...patch }));
      if (!persist) return;
      try {
        localStorage.setItem(KEY, JSON.stringify(store.get()));
      } catch {
        // Not remembered; the choice still applies until the window closes.
      }
    },
  };
}

/** The settings controller. */
export type VoiceSettingsStore = ReturnType<typeof createVoiceSettings>;
