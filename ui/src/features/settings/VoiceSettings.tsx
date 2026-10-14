/**
 * Settings, "Voice": microphone, dictation language, send after dictation (off by
 * default), narration on/off (off by default) and the narration voice; includes voice chat settings
 * (`VoiceChatSettings`). The voices list comes from Mistral through the core (`speech` `voices`, read-only,
 * key from the vault) and is read when the voice list is first used (opening Settings with a locked vault
 * must not raise the unlock prompt).
 * Depends on: AppContext, the voice hooks and settings store, features/voice/pcmCapture
 * (`listMicrophones`). Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: every line says what the choice does and what leaves the computer; a failure to list
 * voices or microphones is shown as text, never swallowed; changing a choice never starts anything.
 */
import { useEffect, useState } from "react";

import { Selector } from "../../components/Selector";
import { toCallError } from "../../protocol/client/types";
import type { VoiceView } from "../../protocol/core";
import { DICTATION_LANGUAGES } from "../../state/voiceSettings";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { listMicrophones } from "../voice/pcmCapture";
import { VoiceChatSettings } from "./VoiceChatSettings";
import { useVoiceSettings } from "../voice/hooks";

/** The section (nothing without a core). */
export function VoiceSettings() {
  const { core, voice } = useApp();
  const keyBlocked = useKeyReadiness() !== null;
  const micId = useVoiceSettings((s) => s.micId);
  const language = useVoiceSettings((s) => s.language);
  const narration = useVoiceSettings((s) => s.narration);
  const autosend = useVoiceSettings((s) => s.autosend);
  const voiceId = useVoiceSettings((s) => s.voice);
  const [mics, setMics] = useState<{ id: string; label: string }[]>([]);
  const [voices, setVoices] = useState<VoiceView[] | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    listMicrophones()
      .then((list) => {
        if (live) setMics(list);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  const loadVoices = () => {
    if (core === null || voices !== null || keyBlocked) return;
    setVoices([]);
    core
      .run({ type: "speech", request: { op: "voices" } })
      .then((r) => {
        if (r.type === "speech" && r.result.kind === "voices") setVoices(r.result.voices);
      })
      .catch((thrown: unknown) => {
        setVoices(null);
        setProblem(`Voices could not be listed: ${toCallError(thrown).error.message}`);
      });
  };

  if (voice === null) return null;
  const update = (patch: Parameters<typeof voice.settings.update>[0]) => {
    voice.settings.update(patch);
  };
  return (
    <section aria-labelledby="voice-title">
      <h3 id="voice-title">Voice</h3>
      <div className="check" data-setting="microphone">
        <span>
          Microphone
          <span className="d">
            Names show after you allowed the microphone once. Dictation sends what it hears to
            Mistral speech-to-text with your key.
          </span>
        </span>
        <Selector
          label="Microphone"
          value={micId ?? ""}
          options={[
            { value: "", label: "System default" },
            ...mics.map((m) => ({ value: m.id, label: m.label })),
          ]}
          onChange={(value) => {
            update({ micId: value === "" ? null : value });
          }}
        />
      </div>
      <div className="check" data-setting="dictation-language">
        <span>
          Dictation language
          <span className="d">
            Auto shows the text live while you speak. A fixed language is more accurate when you
            know it, and the text comes when you stop (live text cannot be told the language).
          </span>
        </span>
        <Selector
          label="Dictation language"
          value={language ?? ""}
          options={[
            { value: "", label: "Auto (live text)" },
            ...DICTATION_LANGUAGES.map((l) => ({ value: l.code, label: l.label })),
          ]}
          onChange={(value) => {
            update({ language: value === "" ? null : value });
          }}
        />
      </div>
      <KeyNotice
        inSettings
        message="Dictation needs your Mistral key before it can turn speech into text."
      />
      <div className="check" data-setting="send-after-dictation">
        <span>
          Send after dictation
          <span className="d">
            Off by default. When on, the message is sent as soon as the dictated text is in the box,
            like Mistral&apos;s Autosend.
          </span>
        </span>
        <button
          type="button"
          className="switch"
          role="switch"
          aria-label="Send after dictation"
          aria-checked={autosend}
          onClick={() => {
            update({ autosend: !autosend });
          }}
        />
      </div>
      <div className="check" data-setting="narration">
        <span>
          Read finished turns aloud
          <span className="d">
            Off by default. When on, each turn that finishes in the session you are looking at is
            summarized by Vibe and read aloud with Mistral text-to-speech: the turn&apos;s text goes
            to Mistral and uses your quota. Never while you dictate.
          </span>
        </span>
        <button
          type="button"
          className="switch"
          role="switch"
          aria-label="Read finished turns aloud"
          aria-checked={narration}
          onClick={() => {
            update({ narration: !narration });
          }}
        />
      </div>
      <KeyNotice
        inSettings
        message="Narration needs your Mistral key to summarize and read finished turns aloud."
      />
      <div className="check" data-setting="narration-voice">
        <span>
          Narration voice
          <span className="d">
            Mistral&apos;s voices and your own saved voices. Voice chat speaks with it too.
          </span>
        </span>
        <Selector
          label="Narration voice"
          value={voiceId ?? ""}
          onOpen={loadVoices}
          options={[
            { value: "", label: "Default (Jane, neutral)" },
            ...(voiceId !== null && !(voices ?? []).some((v) => v.voiceId === voiceId)
              ? [{ value: voiceId, label: voiceId }]
              : []),
            ...(voices ?? []).map((v) => ({
              value: v.voiceId,
              label: v.preset ? v.name : `${v.name} (yours)`,
            })),
          ]}
          onChange={(value) => {
            update({ voice: value === "" ? null : value });
          }}
        />
      </div>
      {problem === null ? null : <p className="key-line">{problem}</p>}
      <VoiceChatSettings />
    </section>
  );
}
