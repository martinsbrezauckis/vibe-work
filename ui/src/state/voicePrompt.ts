/** One permission answer inside voice chat. The core owns codes, policy, recognition and settlement. */
import type { SpeechRequest, SpeechResult } from "../protocol/core";
import type { CoreController } from "./coreStore";
import type { Accessibility } from "./accessibility";
import { createStore, type Store } from "./createStore";
import { toCallError } from "../protocol/client/types";
import { openPrompts } from "./voiceChatTurns";
import { callbackEntry } from "./askBell";
import { approvalQuestion } from "../features/ask/model";
import { approvalQuestionSaid } from "../features/voice/voiceChatText";
import { heardSentence, VOICE_ANSWER_ENDED } from "./voiceQuestion";
import type { VoicePromptState } from "./voiceTypes";

export type { VoicePromptState };
/**
 * Creates the voice answer for permission prompts in voice chat: adds the answer phrase to the spoken prompt, then
 * sends the recorded answer to the core. Works only with voice answers on, prompts read, English or no language set
 * and nothing blocking.
 */
export function createVoicePrompt(deps: {
  core: CoreController;
  accessibility?: Accessibility | undefined;
  speech: (request: SpeechRequest) => Promise<SpeechResult>;
  owner: () => string | null;
  blocked: () => boolean;
  language: () => string | null;
  interrupted: (message?: string) => void;
  store?: Store<VoicePromptState | null> | undefined;
}) {
  const store = deps.store ?? createStore<VoicePromptState | null>(null);
  let session: string | null = null;
  let serial: Promise<unknown> = Promise.resolve();
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const expired = VOICE_ANSWER_ENDED;
  const endedError = (thrown: unknown) => {
    const error = toCallError(thrown).error;
    return (
      error.message === expired ||
      error.message === "no such dictation (it was stopped, cancelled or ended)" ||
      (error.code === "callbackRefused" && error.message === "Answer on the card.")
    );
  };
  const enabled = () =>
    deps.accessibility?.store.get().voiceAnswers === true &&
    deps.accessibility.store.get().readPrompts !== "never" &&
    (deps.language() === null || deps.language() === "en") &&
    !deps.blocked();
  const cancel = () => {
    generation++;
    const current = store.get();
    store.set(null);
    if (timer !== null) clearTimeout(timer);
    timer = null;
    if (current !== null)
      void deps.speech({ op: "promptCancel", ticket: current.ticket }).catch(() => undefined);
  };
  const clearWindow = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  };
  const ended = () => {
    cancel();
    deps.interrupted(expired);
  };
  const armWindow = (durationMs: number) => {
    clearWindow();
    timer = setTimeout(ended, durationMs);
  };
  const own = async (owner: string | null) => {
    if (owner === session) return serial;
    session = owner;
    serial = serial
      .catch(() => undefined)
      .then(() => deps.speech({ op: "promptSession", sessionId: owner }));
    return serial;
  };
  return {
    store,
    cancel,
    ended,
    endOnError(thrown: unknown, active: boolean) {
      if (!active || !endedError(thrown)) return false;
      ended();
      return true;
    },
    end() {
      cancel();
      void own(null).catch(() => undefined);
    },
    reconcile() {
      const current = store.get();
      if (
        current !== null &&
        (!enabled() || !deps.core.store.get().asks[`callback:${current.callbackId}`])
      ) {
        cancel();
        deps.interrupted();
      }
    },
    async prepare(callbackId: string, text: string): Promise<string> {
      const owner = deps.owner();
      if (!enabled() || owner === null) return text;
      const mine = generation;
      try {
        await own(owner);
        if (mine !== generation || !enabled() || owner !== deps.owner()) return text;
        const r = await deps.speech({ op: "promptPrepare", sessionId: owner, callbackId });
        if (r.kind !== "prompt" || r.ticket === null) return text;
        if (mine !== generation || !enabled() || owner !== deps.owner()) {
          void deps.speech({ op: "promptCancel", ticket: r.ticket }).catch(() => undefined);
          return text;
        }
        const phrase =
          r.number === null
            ? "Say no, or answer on the card"
            : `Say allow ${String(r.number)}, or no`;
        const entry = deps.core.store.get().sessions[owner];
        const asks = Object.values(deps.core.store.get().asks);
        const lead = openPrompts(entry, asks).find((p) => p.callbackId === callbackId)?.lead;
        const spoken =
          (lead ?? text.replace(/ Answer on the card in the chat\.$/, "")) + ` ${phrase}.`;
        const card = callbackEntry(deps.core.store.get(), owner, callbackId);
        const fallback =
          card === null ? null : `${approvalQuestionSaid(approvalQuestion(card))} ${phrase}.`;
        store.set({ callbackId, ticket: r.ticket, phrase, text: spoken, fallback, ready: false });
        return spoken;
      } catch {
        return text;
      }
    },
    /**
     * Speaks `text` (a card's reading). When Mistral's safety filter refuses a permission prompt's text, the short
     * fixed prompt is spoken instead (and repeated from then on), so the answer by voice stays possible.
     */
    async speak(callbackId: string | undefined, text: string, voice: string | null) {
      const request = (said: string): SpeechRequest => ({ op: "speak", text: said, voice });
      try {
        return await deps.speech(request(text));
      } catch (thrown) {
        const current = store.get();
        if (
          current === null ||
          current.callbackId !== callbackId ||
          current.text !== text ||
          current.fallback === null ||
          toCallError(thrown).error.serverCode !== "speechGuardrail"
        )
          throw thrown;
        store.set({ ...current, text: current.fallback });
        return deps.speech(request(current.fallback));
      }
    },
    played(callbackId: string) {
      const current = store.get();
      if (current?.callbackId === callbackId && enabled()) store.set({ ...current, ready: true });
    },
    ready: () => store.get()?.ready === true && enabled(),
    async start(sampleRate: number, language: string | null) {
      const current = store.get();
      if (current?.ready !== true || !enabled()) throw new Error("Answer on the card.");
      const r = await deps.speech({
        op: "promptStart",
        ticket: current.ticket,
        sampleRate,
        language,
      });
      if (r.kind === "started") armWindow(r.answerWindowMs ?? 60_000);
      return r;
    },
    async answer(streamId: string): Promise<{ again: string | null; note: string; said?: string }> {
      const current = store.get();
      if (current === null || !enabled()) return { again: null, note: "Answer on the card." };
      store.set({ ...current, ready: false });
      try {
        const r = await deps.speech({ op: "promptAnswer", ticket: current.ticket, streamId });
        if (store.get()?.ticket !== current.ticket)
          return { again: null, note: "Answer on the card." };
        // A miss carries what the core heard ("" when nothing was): said first, like a question card's miss.
        const heard = r.kind === "promptAnswer" ? r.heard : undefined;
        const lead = heard === undefined ? "" : heardSentence(heard, false);
        if (r.kind === "promptAnswer" && r.status === "ended") {
          cancel();
          return { again: null, note: expired, said: expired };
        }
        if (r.kind === "promptAnswer" && (r.status === "retry" || r.status === "repeat")) {
          clearWindow();
          const retry =
            heard === ""
              ? `${lead} ${current.phrase}.`
              : `${lead} That was not an answer. ${current.phrase}.`.trim();
          return {
            again: r.status === "repeat" ? current.text : retry,
            note: r.status === "retry" ? lead : "",
          };
        }
        cancel();
        return {
          again: null,
          note:
            r.kind === "promptAnswer" && r.status === "allowed"
              ? "Allowed once by voice."
              : r.kind === "promptAnswer" && r.status === "refused"
                ? "Said No by voice."
                : `${lead} Answer on the card.`.trim(),
        };
      } catch (thrown) {
        if (endedError(thrown)) {
          cancel();
          return { again: null, note: expired, said: expired };
        }
        cancel();
        return { again: null, note: "Answer on the card." };
      }
    },
  };
}
