/** Short accessibility preferences page; shared Settings controls backed by the accessibility store. */
import { Selector } from "../../components/Selector";
import { Switch } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import { TEXT_SIZES } from "../../state/accessibility";
import { useApp } from "../shell/AppContext";

export function AccessibilitySettings() {
  const { accessibility } = useApp();
  const s = useStore(accessibility.store, (state) => state);
  return (
    <>
      <section aria-labelledby="a11y-prompts-title">
        <h3 id="a11y-prompts-title">Permission prompts</h3>
        <div className="check" data-setting="a11y-prompt-sound">
          <span>Sound when Vibe asks</span>
          <Selector
            testId="settings.accessibility-prompt-sound"
            label="Sound when Vibe asks"
            value={s.promptSound}
            options={[
              { value: "auto", label: "Auto" },
              { value: "on", label: "On" },
              { value: "off", label: "Off" },
            ]}
            onChange={(promptSound) => {
              accessibility.update({ promptSound });
            }}
          />
        </div>
        <div className="check" data-setting="a11y-read-prompts">
          <span>Read prompts aloud</span>
          <Selector
            testId="settings.accessibility-read-prompts"
            label="Read prompts aloud"
            value={s.readPrompts}
            options={[
              { value: "voiceChat", label: "In voice chat" },
              { value: "always", label: "Always" },
              { value: "never", label: "Never" },
            ]}
            onChange={(readPrompts) => {
              accessibility.update({ readPrompts });
            }}
          />
        </div>
        <div className="check" data-setting="a11y-voice-answer">
          <span>Answer prompts by voice</span>
          <Switch
            data-testid="settings.accessibility-voice-answer"
            aria-label="Answer prompts by voice"
            aria-checked={s.voiceAnswers}
            onClick={() => {
              accessibility.update({ voiceAnswers: !s.voiceAnswers });
            }}
          />
        </div>
      </section>
      <section aria-labelledby="a11y-readers-title">
        <h3 id="a11y-readers-title">Screen readers</h3>
        <div className="check" data-setting="a11y-reply-announce">
          <span>When Vibe finishes a reply</span>
          <Selector
            testId="settings.accessibility-reply"
            label="When Vibe finishes a reply"
            value={s.reply}
            options={[
              { value: "sentence", label: "First sentence" },
              { value: "whole", label: "Whole reply" },
              { value: "short", label: "Only “Vibe finished”" },
            ]}
            onChange={(reply) => {
              accessibility.update({ reply });
            }}
          />
        </div>
      </section>
      <section aria-labelledby="a11y-display-title">
        <h3 id="a11y-display-title">Text, contrast and motion</h3>
        <div className="check" data-setting="a11y-text-size">
          <span>Text size</span>
          <Selector
            testId="settings.accessibility-text-size"
            label="Text size"
            value={String(s.textSize)}
            options={TEXT_SIZES.map((size) => ({ value: String(size), label: `${String(size)}%` }))}
            onChange={(size) => {
              accessibility.update({ textSize: Number(size) });
            }}
          />
        </div>
        <div className="check" data-setting="a11y-contrast">
          <span>Contrast</span>
          <Selector
            testId="settings.accessibility-contrast"
            label="Contrast"
            value={s.contrast}
            options={[
              { value: "system", label: "Follow the system" },
              { value: "more", label: "More contrast" },
            ]}
            onChange={(contrast) => {
              accessibility.update({ contrast });
            }}
          />
        </div>
        <div className="check" data-setting="a11y-motion">
          <span>Motion</span>
          <Selector
            testId="settings.accessibility-motion"
            label="Motion"
            value={s.motion}
            options={[
              { value: "system", label: "Follow the system" },
              { value: "reduce", label: "Reduce" },
            ]}
            onChange={(motion) => {
              accessibility.update({ motion });
            }}
          />
        </div>
      </section>
    </>
  );
}
