/**
 * Setup step "the vault is open but holds no Mistral key": use a key already saved in the vault with
 * one click, take the key Vibe saved itself, or paste one. The pasted key goes to the core through
 * the one-way command and the field is cleared at once.
 * Depends on: AppContext, useKeys, fields, VibeKeyOffer, ConsoleLink. Called by: SetupScreen,
 * ResultStep (to replace a key that failed), KeySettings.
 */
import { useState } from "react";

import { useApp } from "../shell/AppContext";
import { ConsoleLink } from "./ConsoleLink";
import { Problem, SecretField } from "./fields";
import { UseVibeKey } from "./VibeKeyOffer";
import { useKeyUi } from "./useKeys";

/** The step. `offers` false hides the saved/Vibe key offers (replacing a key that failed). */
export function KeyStep({
  offers = true,
  label,
  onSaved,
}: {
  offers?: boolean;
  label?: string;
  /** Called after the key was stored (Settings closes its replace form). */
  onSaved?: () => void;
}) {
  const { keys } = useApp();
  const candidates = useKeyUi((s) => s.candidates);
  const vibeKeys = useKeyUi((s) => s.vibeKeys);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const [key, setKey] = useState("");

  const save = () => {
    if (keys === null || key.trim() === "") return;
    const value = key;
    setKey("");
    void keys.saveKey(value).then((ok) => {
      if (ok) onSaved?.();
    });
  };

  return (
    <div className="setup-step">
      {offers && candidates.length > 0 && (
        <div className="key-offer">
          <p>Your vault already holds a Mistral key.</p>
          {candidates.map((c) => (
            <div data-item-id={c.itemId} className="key-saved" key={c.itemId}>
              <span>{c.label}</span>
              <button
                data-testid="setup.key-step-use-saved"
                type="button"
                className="btn-primary"
                disabled={busy}
                onClick={() => {
                  void keys?.useSaved(c.itemId);
                }}
              >
                Use the saved key
              </button>
            </div>
          ))}
        </div>
      )}
      {offers &&
        vibeKeys.map((found) => (
          <UseVibeKey
            key={found.source}
            found={found}
            busy={busy}
            onUse={(source) => {
              void keys?.importVibeKey(source);
            }}
          />
        ))}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <SecretField
          label={label ?? "Paste your Mistral API key"}
          testId="setup.key-input"
          value={key}
          onChange={setKey}
          placeholder="Your key"
          autoFocus={offers && candidates.length === 0 && vibeKeys.length === 0}
        />
        <ConsoleLink />
        <Problem text={problem?.text ?? null} />
        <div className="key-actions">
          <button
            type="submit"
            className="btn-primary"
            data-testid="setup.save"
            disabled={busy || key.trim() === ""}
          >
            {busy ? "Saving..." : "Save and test the key"}
          </button>
        </div>
      </form>
    </div>
  );
}
