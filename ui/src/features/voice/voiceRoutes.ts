/**
 * The `ui/voice` UI-control route. The Rust side lists the route name
 * (`crates/vw-host/src/host.rs` SHELL_ROUTES). Every action calls the same controller function the mic
 * button or the narration bar calls, with the Debug API as the actor:
 * - `read` (default): dictation (phase, stream, mode, seconds, live text, note, error, last text) and
 * narration (enabled, phase, summary, queue, last played);
 * - `start {sessionId?}`: while the previous dictation is still being made into text, the start is queued (a
 * second `start` cancels the queue); `dictation.queued` (true|false) in every reply says so. Otherwise the
 * core's `speech start` runs with the Debug API origin (a label; no native confirmation); the text
 * later goes in with the external-client mark. A queued start that runs after this request was answered goes
 * out as the window's (the Debug API origin requires a request being served). The
 * answer comes once the dictation records, failed, or waits for the microphone (`micPending`, an OS prompt), so an unanswered prompt never holds the other `ui/*` requests;
 * - `stop` (waits for the text), `cancel`;
 * - `readAloud {sessionId?, text?}`: queues a one-time reading through the core without changing settings.
 * - `narrateLast {sessionId?}`: narrates the session's last finished turn; `stopNarration`, `skipNarration`.
 * - `settings {narration?, voice?, language?, micId?, voiceChatAutoEnd?, voiceChatPauseMs?}`: the Settings screen's
 * voice choices (the Debug API acts as the user);
 * - Voice chat: `voiceChat {on: true|false, sessionId?}` turns it on for the session (moving it from
 * another) or off; `voiceChatEnd` ends the user's turn now (the button's press while listening). `read` answers
 * `voiceChat {on, owner, phase, micPending, partial, heard, endedBy, lastSpoken {text, chars, seconds, kind},
 * spoken, note, error, autoEnd, pauseMs}`. Recorded audio in tests comes from the browser's fake microphone
 * (voice_e2e), which plays its file again at each capture.
 * Depends on: AppContext, uiControl. Called by: useShellWiring.
 * Invariants: bodies are validated (`invalidParams`); `sessionId` defaults to the selected session.
 */
import { toCallError } from "../../protocol/client/types";
import { UiControlError } from "../../state/uiControl";
import type { App } from "../shell/AppContext";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

/** What `read` answers. */
function readVoice(app: App): unknown {
  const voice = app.voice;
  if (voice === null) return null;
  const d = voice.dictation.store.get();
  const n = voice.narration.store.get();
  return {
    dictation: {
      phase: d.phase,
      sessionId: d.sessionId,
      streamId: d.streamId,
      mode: d.mode,
      by: d.by,
      seconds: d.seconds,
      maxSeconds: d.maxSeconds,
      partial: d.partial,
      note: d.note,
      error: d.error,
      lastText: d.lastText,
      queued: d.queued,
      micPending: d.micPending,
      permissionWait: d.permissionWait,
    },
    voiceChat: readVoiceChat(voice),
    narration: {
      enabled: voice.settings.store.get().narration,
      phase: n.phase,
      summary: n.current?.summary ?? null,
      queue: n.queue.length,
      last: n.last,
      note: n.note,
    },
  };
}

/** What `read` answers about voice chat. */
function readVoiceChat(voice: NonNullable<App["voice"]>): unknown {
  const c = voice.voiceChat.store.get();
  const settings = voice.settings.store.get();
  return {
    on: c.on,
    owner: c.owner,
    phase: c.phase,
    micPending: c.micPending,
    partial: c.partial,
    heard: c.heard,
    endedBy: c.endedBy,
    lastSpoken: c.lastSpoken,
    spoken: c.spoken,
    note: c.note,
    error: c.error,
    autoEnd: settings.voiceChatAutoEnd,
    pauseMs: settings.voiceChatPauseMs,
    latency: c.latency,
  };
}

/** `voiceChat {on, sessionId?}`: on for the session, or off. */
function switchVoiceChat(
  voice: NonNullable<App["voice"]>,
  input: Record<string, unknown>,
  sessionId: string | null,
): void {
  const on = input["on"];
  if (on === false) {
    voice.voiceChat.turnOff();
    return;
  }
  if (on !== true) throw new UiControlError("invalidParams", "voiceChat needs on: true or false");
  if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
  const refused = voice.voiceChat.turnOn(sessionId);
  if (refused !== null) throw new UiControlError("denied", refused);
}

/**
 * Starts a dictation and answers once it records, failed, or waits for the microphone. The UI-control
 * loop serves one request at a time, so awaiting an OS permission prompt here held every later `ui/*` request
 * (measured on macOS: 10 s timeouts while the prompt was up). The core's `speech start` has run by then (the
 * Debug API origin applies to it); opening the microphone goes on in the window. Poll `read` for `recording`.
 */
async function startUntilMicPending(
  dictation: NonNullable<App["voice"]>["dictation"],
  sessionId: string,
): Promise<void> {
  const started = dictation.start(sessionId, "debug-api");
  let unsubscribe: () => void = () => undefined;
  const pending = new Promise<void>((resolve) => {
    const check = (): void => {
      if (dictation.store.get().micPending) resolve();
    };
    unsubscribe = dictation.store.subscribe(check);
    check();
  });
  try {
    await Promise.race([started, pending]);
  } finally {
    unsubscribe();
  }
}

/** Runs one `ui/voice` request. */
async function handleVoice(app: App, body: unknown): Promise<unknown> {
  const input = asObject(body);
  const voice = app.voice;
  if (voice === null) throw new UiControlError("serverUnavailable", "the app has no core");
  const given = input["sessionId"];
  const sessionId =
    typeof given === "string" && given !== "" ? given : app.shell.store.get().selected;
  const action = input["action"] ?? "read";
  try {
    switch (action) {
      case "read":
        return readVoice(app);
      case "start":
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
        // While the previous text is being made, `start` queues (a second one cancels the queue): the reply's
        // `dictation.queued` says which happened.
        if (voice.dictation.store.get().phase === "transcribing")
          voice.dictation.queueStart(sessionId, "debug-api");
        else await startUntilMicPending(voice.dictation, sessionId);
        return readVoice(app);
      case "stop":
        await voice.dictation.stop();
        return readVoice(app);
      case "cancel":
        voice.dictation.cancel();
        return readVoice(app);
      case "readAloud": {
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
        const text = input["text"];
        if (text !== undefined && typeof text !== "string")
          throw new UiControlError("invalidParams", "Reading aloud needs text in plain words.");
        const answer = await app.core?.run({
          type: "speech",
          request: { op: "readAloud", sessionId, text: text ?? null },
        });
        return answer?.type === "speech" ? answer.result : null;
      }
      case "narrateLast":
        if (sessionId === null) throw new UiControlError("invalidParams", "no session is selected");
        await voice.narration.narrateLast(sessionId);
        return readVoice(app);
      case "stopNarration":
        voice.narration.stopAll();
        return readVoice(app);
      case "skipNarration":
        voice.narration.skip();
        return readVoice(app);
      case "voiceChat":
        switchVoiceChat(voice, input, sessionId);
        return readVoice(app);
      case "voiceChatEnd":
        await voice.voiceChat.endTurn();
        return readVoice(app);
      case "settings": {
        // the Debug API acts as the user, so it changes the voice settings like the Settings
        // screen does (narration on/off, voice, dictation language, microphone id); unknown values fall back
        // to the defaults exactly as a stored value would (`parseVoiceSettings`).
        const patch: Record<string, unknown> = {};
        for (const key of [
          "narration",
          "voice",
          "language",
          "micId",
          "voiceChatAutoEnd",
          "voiceChatPauseMs",
        ] as const)
          if (key in input) patch[key] = input[key];
        voice.settings.update(patch);
        return readVoice(app);
      }
      default:
        throw new UiControlError("invalidParams", `unknown action ${JSON.stringify(action)}`);
    }
  } catch (thrown) {
    if (thrown instanceof UiControlError) throw thrown;
    const error = toCallError(thrown).error;
    throw new UiControlError(error.code, error.message);
  }
}

/** Registers the route; returns the function that removes it. */
export function registerVoiceRoutes(app: App): () => void {
  return app.uiControl.register("voice", (body) => handleVoice(app, body));
}
