/**
 * The last setup step: the test call's outcome in plain words. "Key works. N models available" or
 * what 401 / 403 / a network failure means and what to do; for a refused key the user can paste
 * another one right here. Also offers to remove Vibe's own copy after an import (never silently) and
 * shows why "remember on this computer" did not work, if it did not.
 * Depends on: AppContext, useKeys, KeyStep, fields. Called by: SetupScreen.
 */
import { useState } from "react";

import { placeOf } from "./VibeKeyOffer";
import { useApp } from "../shell/AppContext";
import { KeyStep } from "./KeyStep";
import { useKeyUi } from "./useKeys";

/** The step. */
export function ResultStep() {
  const { keys } = useApp();
  const test = useKeyUi((s) => s.test);
  const status = useKeyUi((s) => s.status);
  const removable = useKeyUi((s) => s.removable);
  const vibeKeys = useKeyUi((s) => s.vibeKeys);
  const notice = useKeyUi((s) => s.notice);
  const busy = useKeyUi((s) => s.busy);
  const [replacing, setReplacing] = useState(false);
  const copy = vibeKeys.find((k) => k.source === removable);

  return (
    <div
      className="setup-step"
      aria-live="polite"
      data-testid="setup-result"
      data-phase={test.phase}
    >
      {test.phase === "running" && <p>Testing the key...</p>}
      {test.phase === "idle" && <p>The key is saved. Test it to make sure Mistral accepts it.</p>}
      {test.phase === "ok" && (
        <p className="key-success" role="status">
          Key works. {test.models} {test.models === 1 ? "model is" : "models are"} available.
        </p>
      )}
      {test.phase === "failed" && (
        <p className="key-problem" role="alert">
          {test.text}
        </p>
      )}
      {status?.rememberNote != null && (
        <p className="key-field-hint">
          This computer could not remember the password ({status.rememberNote}). You will type it
          again next time.
        </p>
      )}
      {removable !== null && (
        <div className="key-offer">
          <p>
            Vibe still has its own copy of the key
            {copy === undefined ? "" : ` in ${placeOf(copy)}`}. Remove it so only the vault holds
            the key?
          </p>
          <button
            type="button"
            className="tb-btn"
            disabled={busy}
            onClick={() => {
              void keys?.removeVibeCopy(removable);
            }}
          >
            Remove Vibe&apos;s copy
          </button>
        </div>
      )}
      {notice !== null && <p className="key-field-hint">{notice}</p>}
      {replacing && test.phase !== "ok" && <KeyStep offers={false} label="Paste another key" />}
      <div className="key-actions">
        {test.phase === "ok" && (
          <button
            type="button"
            className="btn-primary"
            data-testid="setup-continue"
            onClick={() => {
              keys?.finish();
            }}
          >
            Continue
          </button>
        )}
        {(test.phase === "failed" || test.phase === "idle") && (
          <>
            <button
              type="button"
              className="btn-primary"
              data-testid="setup-retry"
              disabled={busy}
              onClick={() => {
                void keys?.testKey();
              }}
            >
              Try again
            </button>
            {!replacing && (
              <button
                type="button"
                className="tb-btn"
                onClick={() => {
                  setReplacing(true);
                }}
              >
                Use a different key
              </button>
            )}
            <button
              type="button"
              className="tb-btn"
              onClick={() => {
                keys?.finish();
              }}
            >
              Continue anyway
            </button>
          </>
        )}
      </div>
    </div>
  );
}
