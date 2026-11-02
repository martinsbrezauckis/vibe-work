/** Remembered accessibility choices. Used by Settings, shell wiring, prompt audio and reply announcements. */
import { createStore } from "./createStore";

export const TEXT_SIZES = [100, 110, 125, 150, 175, 200] as const;
export interface AccessibilitySettings {
  promptSound: "auto" | "on" | "off";
  readPrompts: "voiceChat" | "always" | "never";
  voiceAnswers: boolean;
  reply: "sentence" | "whole" | "short";
  textSize: number;
  contrast: "system" | "more";
  motion: "system" | "reduce";
}
export const ACCESSIBILITY_DEFAULTS: AccessibilitySettings = {
  promptSound: "auto",
  readPrompts: "voiceChat",
  voiceAnswers: false,
  reply: "sentence",
  textSize: 100,
  contrast: "system",
  motion: "system",
};
const KEY = "vw.accessibility";

/** Validate each field independently so an old preference cannot turn on voice answers. */
export function parseAccessibility(raw: unknown): AccessibilitySettings {
  const r = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  return {
    promptSound:
      r["promptSound"] === "on" || r["promptSound"] === "off" ? r["promptSound"] : "auto",
    readPrompts:
      r["readPrompts"] === "always" || r["readPrompts"] === "never"
        ? r["readPrompts"]
        : "voiceChat",
    voiceAnswers: r["voiceAnswers"] === true,
    reply: r["reply"] === "whole" || r["reply"] === "short" ? r["reply"] : "sentence",
    textSize: TEXT_SIZES.find((size) => size === r["textSize"]) ?? 100,
    contrast: r["contrast"] === "more" ? "more" : "system",
    motion: r["motion"] === "reduce" ? "reduce" : "system",
  };
}

export function createAccessibility(persist: boolean) {
  let initial = ACCESSIBILITY_DEFAULTS;
  if (persist) {
    try {
      initial = parseAccessibility(JSON.parse(localStorage.getItem(KEY) ?? "null"));
    } catch {
      /* Storage may be unavailable. */
    }
  }
  const store = createStore(initial);
  return {
    store,
    update(patch: Partial<AccessibilitySettings>): void {
      store.set(parseAccessibility({ ...store.get(), ...patch }));
      if (persist) {
        try {
          localStorage.setItem(KEY, JSON.stringify(store.get()));
        } catch {
          /* The choice still applies for this window. */
        }
      }
    },
  };
}
export type Accessibility = ReturnType<typeof createAccessibility>;
