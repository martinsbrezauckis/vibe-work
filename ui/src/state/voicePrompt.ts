/** One permission answer inside voice chat. The core owns codes, policy, recognition and settlement. */
import type { SpeechRequest, SpeechResult } from "../protocol/core";
import type { CoreController } from "./coreStore";
import type { Accessibility } from "./accessibility";
import { createStore } from "./createStore";

export interface VoicePromptState {
  callbackId: string;
  ticket: string;
  phrase: string;
  text: string;
  ready: boolean;
}
export function createVoicePrompt(deps: {
  core: CoreController;
  accessibility?: Accessibility | undefined;
  speech: (request: SpeechRequest) => Promise<SpeechResult>;
  owner: () => string | null;
  blocked: () => boolean;
  language: () => string | null;
  interrupted: () => void;
}) {
  const store = createStore<VoicePromptState | null>(null);
  let session: string | null = null;
  let serial: Promise<unknown> = Promise.resolve();
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
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
        const spoken = text.replace(/ Answer on the card in the chat\.$/, "") + ` ${phrase}.`;
        store.set({ callbackId, ticket: r.ticket, phrase, text: spoken, ready: false });
        return spoken;
      } catch {
        return text;
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
      if (timer === null)
        timer = setTimeout(() => {
          cancel();
          deps.interrupted();
        }, 60_000);
      return r;
    },
    async answer(streamId: string): Promise<{ again: string | null; note: string }> {
      const current = store.get();
      if (current === null || !enabled()) return { again: null, note: "Answer on the card." };
      store.set({ ...current, ready: false });
      try {
        const r = await deps.speech({ op: "promptAnswer", ticket: current.ticket, streamId });
        if (store.get()?.ticket !== current.ticket)
          return { again: null, note: "Answer on the card." };
        if (r.kind === "promptAnswer" && (r.status === "retry" || r.status === "repeat")) {
          return {
            again:
              r.status === "repeat" ? current.text : `That was not an answer. ${current.phrase}.`,
            note: "",
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
                : "Answer on the card.",
        };
      } catch {
        cancel();
        return { again: null, note: "Answer on the card." };
      }
    },
  };
}
