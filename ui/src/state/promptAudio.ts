/** Arrival sounds and optional prompt readings. Uses the shared speech API/player; never replies to a prompt. */
import type { Ask } from "../protocol/core";
import type { CoreController } from "./coreStore";
import type { Accessibility } from "./accessibility";
import type { VoiceSettingsStore } from "./voiceSettings";
import type { Player } from "../features/voice/audioPlayer";
import { promptTone } from "../features/voice/promptTone";
import { questionsOf } from "../features/transcript/model/callbacks";
import { callbackEntry, askWordsFor } from "./askBell";
import { speakable } from "../features/voice/speechText";
import { buttonsFor } from "../features/ask/model";
import { announce } from "./announcer";

/** The same question and choice labels as the card; native asks expose only value-free metadata. */
export function promptWords(core: CoreController, ask: Ask): string {
  const state = core.store.get();
  const entry = ask.id.startsWith("callback:")
    ? callbackEntry(state, ask.session ?? "", ask.id.slice(9))
    : null;
  if (ask.kind === "user_input" && entry !== null) {
    return speakable(
      questionsOf(entry)
        .questions.map((q) => `${q.question} ${q.options.map((o) => o.label).join(", ")}.`)
        .join(" ") + " Answer on the card.",
      1800,
    );
  }
  const words = askWordsFor(state, ask);
  const choices = buttonsFor(ask.offers, null)
    .map((button) => button.label)
    .join(", ");
  return speakable(
    `Vibe asks: ${words.question} ${words.summary}. ${choices}. Answer ${ask.surface === "native" ? "in the confirmation window" : "on the card"}.`,
    1800,
  );
}

export function createPromptAudio(deps: {
  core: CoreController;
  accessibility: Accessibility;
  settings: VoiceSettingsStore;
  player: Player;
  signal: Player;
  busy: () => boolean;
  owns: (session: string | null) => boolean;
}) {
  let generation = 0,
    playing = false;
  let currentAsk: string | null = null;
  const pending: Ask[] = [];
  const { core, accessibility, settings, player, signal } = deps;
  const open = (id: string) => core.store.get().asks[id] !== undefined;
  const drain = async () => {
    if (playing || deps.busy()) return;
    const ask = pending.shift();
    if (!ask) return;
    if (!open(ask.id) || accessibility.store.get().readPrompts !== "always") {
      void drain();
      return;
    }
    playing = true;
    currentAsk = ask.id;
    const mine = generation;
    try {
      const result = await core.run({
        type: "speech",
        request: { op: "speak", text: promptWords(core, ask), voice: settings.store.get().voice },
      });
      if (
        mine !== generation ||
        !open(ask.id) ||
        deps.busy() ||
        accessibility.store.get().readPrompts !== "always"
      )
        return;
      if (result.type === "speech" && result.result.kind === "speech") {
        const audio = Uint8Array.from(atob(result.result.audio), (c) => c.charCodeAt(0));
        await player.play(audio);
      }
    } catch {
      announce("Prompt audio unavailable.");
    } finally {
      playing = false;
      currentAsk = null;
      if (mine === generation) void drain();
    }
  };
  const arrived = async (ask: Ask) => {
    const mine = generation;
    const mode = accessibility.store.get().promptSound;
    let sound = mode === "on";
    if (mode === "auto") {
      try {
        const result = await core.run({ type: "speech", request: { op: "screenReader" } });
        sound =
          result.type === "speech" &&
          result.result.kind === "screenReader" &&
          result.result.active === true;
      } catch {
        /* Unknown reader state does not force a sound. */
      }
    }
    if (mine !== generation || !open(ask.id)) return;
    if (sound && accessibility.store.get().promptSound !== "off") {
      try {
        await signal.play(promptTone());
      } catch {
        announce("Prompt sound unavailable.");
      }
    }
    if (mine !== generation || !open(ask.id) || deps.owns(ask.session)) return;
    if (accessibility.store.get().readPrompts === "always") {
      pending.push(ask);
      void drain();
    }
  };
  return {
    /** Reads queued prompts when dictation or voice chat has finished using the output. */
    resume: () => {
      void drain();
    },
    stop(): void {
      generation++;
      pending.length = 0;
      if (playing) player.stop();
      signal.stop();
    },
    watch(): () => void {
      const seen = new Set(Object.keys(core.store.get().asks));
      const stop = core.listen((event) => {
        if (event.type === "askOpened" && !seen.has(event.ask.id)) {
          seen.add(event.ask.id);
          if (seen.size > 1000) seen.delete(seen.values().next().value ?? "");
          void arrived(event.ask);
        }
      });
      const unwatch = accessibility.store.subscribe(() => {
        if (accessibility.store.get().readPrompts === "never") {
          pending.length = 0;
          if (playing) player.stop();
        }
      });
      const unlisten = core.store.subscribe(() => {
        if (currentAsk !== null && !open(currentAsk)) player.stop();
      });
      return () => {
        stop();
        unwatch();
        unlisten();
        generation++;
        pending.length = 0;
        if (playing) player.stop();
        signal.stop();
      };
    },
  };
}
