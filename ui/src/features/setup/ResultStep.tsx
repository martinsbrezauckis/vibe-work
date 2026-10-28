/**
 * The last setup step: the test call's outcome in plain words. "Key works. N models available" or
 * what 401 / 403 / a network failure means and what to do; for a refused key the user can paste
 * another one right here. Also offers to remove Vibe's own copy after an import (never silently) and
 * shows why the credential store could not keep the vault's key, if it could not.
 * Depends on: AppContext, useKeys, KeyStep, fields. Called by: SetupScreen.
 */
import { Button } from "../../components/Controls";
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
      {test.phase === "running" && <p>Testing the key…</p>}
      {test.phase === "idle" && <p>The key is saved. Test it to make sure Mistral accepts it.</p>}
      {test.phase === "ok" && (
        <p className="key-success" role="status">
          Key works.
        </p>
      )}
      {test.phase === "failed" && (
        <p className="key-problem" role="alert">
          {test.text}
        </p>
      )}
      {status?.rememberNote != null && (
        <p className="key-field-hint">
          This computer&apos;s credential store could not keep the vault&apos;s key (
          {status.rememberNote}). You will type the password at the next start.
        </p>
      )}
      {removable !== null && (
        <div className="key-offer">
          <p>
            Vibe still has its own copy of the key
            {copy === undefined ? "" : ` in ${placeOf(copy)}`}. Remove it so only the vault holds
            the key?
          </p>
          <Button
            data-testid="setup.result-step-remove-vibe-copy"
            type="button"
            shape="outlined"
            variant="danger"
            disabled={busy}
            onClick={() => {
              void keys?.removeVibeCopy(removable);
            }}
          >
            Remove Vibe&apos;s copy
          </Button>
        </div>
      )}
      {notice !== null && <p className="key-field-hint">{notice}</p>}
      {replacing && test.phase !== "ok" && <KeyStep offers={false} label="Paste another key" />}
      <div className="key-actions">
        {test.phase === "ok" && (
          <Button
            type="button"
            appearance="dialog"
            data-testid="setup.continue"
            onClick={() => {
              keys?.finish();
            }}
          >
            Continue
          </Button>
        )}
        {(test.phase === "failed" || test.phase === "idle") && (
          <>
            <Button
              type="button"
              appearance="dialog"
              data-testid="setup.retry"
              disabled={busy}
              onClick={() => {
                void keys?.testKey();
              }}
            >
              Try again
            </Button>
            {!replacing && (
              <Button
                data-testid="setup.result-step-set-replacing"
                type="button"

                onClick={() => {
                  setReplacing(true);
                }}
              >
                Use a different key
              </Button>
            )}
            <Button
              data-testid="setup.result-step-finish"
              type="button"

              onClick={() => {
                keys?.finish();
              }}
            >
              Continue anyway
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
