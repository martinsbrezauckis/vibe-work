/** Search anchors for Accessibility; consumed by the shared Settings catalogue. */
import type { SettingEntry } from "./catalog";
export const accessibilityCatalog: readonly SettingEntry[] = [
  ["a11y-prompt-sound", "Sound when Vibe asks", "sound permission prompt"],
  ["a11y-read-prompts", "Read prompts aloud", "speak voice question"],
  ["a11y-voice-answer", "Answer prompts by voice", "allow microphone voice chat"],
  [
    "a11y-reply-announce",
    "When Vibe finishes a reply",
    "screen reader VoiceOver Narrator Orca NVDA",
  ],
  ["a11y-text-size", "Text size", "zoom bigger text Ctrl + -"],
  ["a11y-contrast", "Contrast", "low vision contrast colours colors"],
  ["a11y-motion", "Motion", "reduce animation system"],
].map(([id = "", label = "", keywords = ""]) => ({
  id,
  label,
  keywords,
  category: "accessibility",
  description: "",
}));
