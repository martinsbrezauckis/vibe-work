/**
 * The composer's microphone button (adapted from ShellX: its state machine and labels, without MediaRecorder or xAI). One gesture for both modes: **hold** (released after
 * 400 ms stops and inserts the text) and **press** (a short press starts; the next press stops). Keyboard:
 * Enter/Space toggle; Escape cancels (wired in `hooks.ts`).
 * Depends on: AppContext, the voice hooks, Icon. Called by: Composer (through the tool row's `mic` slot).
 * Invariants: render and event wiring only; the controller decides (`state/dictation`); while another
 * session dictates, or while voice chat is on (one microphone), this button is disabled. While the previous text is still being made ("Finishing…") a press
 * queues the next dictation and a second press cancels it.
 */
import { type ReactElement, useRef } from "react";

import { Icon } from "../../components/Icon";
import { Walker } from "../../components/Walker";
import { formatKeys } from "../../state/shortcuts";
import { useApp } from "../shell/AppContext";
import { useKeyReadiness } from "../shell/KeyNotice";
import { DICTATE_KEYS, HOLD_MS, clock, useDictation, useVoiceChat } from "./hooks";

/** The button for `sessionId`'s composer. */
export function MicButton({
  sessionId,
  disabled = false,
}: {
  sessionId: string;
  disabled?: boolean;
}): ReactElement | null {
  const voice = useApp().voice;
  const keyBlocked = useKeyReadiness() !== null;
  const phase = useDictation((s) => s.phase);
  const owner = useDictation((s) => s.sessionId);
  const seconds = useDictation((s) => s.seconds);
  const error = useDictation((s) => s.error);
  const queued = useDictation((s) => s.queued);
  const press = useRef<{ at: number; started: boolean } | null>(null);
  // voice chat and dictation share the microphone; while voice chat is on, dictation waits.
  const voiceChatOn = useVoiceChat((s) => s.on);
  if (voice === null) return null;
  const { dictation } = voice;
  const mine = (owner === null || owner === sessionId) && !voiceChatOn;
  const active = mine && (phase === "recording" || phase === "starting");
  // While the previous text is being made ("Finishing…") a press is not lost: it queues the next dictation.
  const finishing = mine && phase === "transcribing";
  const keys = formatKeys(DICTATE_KEYS);
  const label = keyBlocked
    ? "Dictation needs a usable Mistral key. Open Voice settings to restore it."
    : voiceChatOn
      ? "Dictate: unavailable while voice chat is on"
      : !mine
        ? "Dictate: unavailable while dictating in another session"
        : phase === "starting"
          ? `Dictate: starting the microphone; press to stop (${keys}); Escape to cancel`
          : phase === "recording"
            ? `Dictate: recording ${clock(seconds)}; release or press to stop (${keys}); Escape to cancel`
            : phase === "transcribing"
              ? queued
                ? `Dictate: finishing; press to cancel the next dictation (${keys})`
                : `Dictate: finishing; press to dictate again right after (${keys})`
              : phase === "error" && error !== null
                ? `Dictate (${keys}). Last try: ${error}`
                : `Dictate: hold to talk, or press to start and stop (${keys})`;
  return (
    <button
      type="button"
      className={`ibtn mic mic-${mine ? phase : "idle"}`}
      aria-label={label}
      title={label}
      aria-pressed={active}
      disabled={disabled || !mine || (keyBlocked && !active)}
      data-queued={finishing && queued ? "true" : undefined}
      data-testid="voice.mic-button"
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        if (phase === "transcribing") {
          press.current = null;
          dictation.queueStart(sessionId);
        } else if (phase === "idle" || phase === "error") {
          press.current = { at: Date.now(), started: true };
          void dictation.start(sessionId);
        } else {
          press.current = { at: Date.now(), started: false };
        }
      }}
      onPointerUp={() => {
        const p = press.current;
        press.current = null;
        if (p === null) return;
        if (!p.started || Date.now() - p.at >= HOLD_MS) void dictation.stop();
      }}
      onClick={(e) => {
        // Pointer clicks were handled on down/up; this is the keyboard (detail 0).
        if (e.detail !== 0) return;
        if (phase === "transcribing") dictation.queueStart(sessionId);
        else if (phase === "idle" || phase === "error") void dictation.start(sessionId);
        else if (active) void dictation.stop();
      }}
    >
      <Icon name={active ? "square" : "mic"} />
      {/* while the text is being made, the pixel walker (work in progress) sits on the mic; a queued next
          dictation adds a ring. */}
      {finishing && <Walker size="badge" />}
    </button>
  );
}
