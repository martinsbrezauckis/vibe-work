/**
 * The card that asks for a new default model when Vibe no longer offers the Settings default: the gone model's name,
 * one button per model Vibe offers, and Later.
 * Depends on: AppContext (the default-model controller), createStore, useModalFocus, Controls, vocabulary.
 * Called by: Shell.
 * Invariants: shown exactly while the controller names a gone default; modal (`aria-modal`), Escape and the scrim are
 * Later; nothing is written until a model is picked.
 */
import { type ReactElement, useRef } from "react";

import { Button } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import { createStore, useStore } from "../../state/createStore";
import type { DefaultModelState } from "../../state/defaultModel";
import { TEXT, wording } from "../../vocabulary";
import { useApp } from "../shell/AppContext";

const NONE = createStore<DefaultModelState>({ gone: null, models: [], saving: false, error: null });

/** The card, or nothing. */
export function DefaultModelDialog(): ReactElement | null {
  const controller = useApp().defaultModel;
  const state = useStore(controller?.store ?? NONE, (s) => s);
  const dialog = useRef<HTMLDivElement>(null);
  const open = controller !== null && state.gone !== null;
  const later = () => {
    controller?.later();
  };
  useModalFocus(open, dialog, later);
  if (controller === null || state.gone === null) return null;
  return (
    <>
      <div data-testid="setup.default-model-scrim" className="scrim" onClick={later} />
      <div
        className="modal default-model-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="default-model-title"
        data-testid="setup.default-model-dialog"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2 id="default-model-title">{TEXT.defaultModelChoose}</h2>
        </header>
        <section>
          <p>{wording(TEXT.defaultModelGone, { model: state.gone.label })}</p>
          <div className="default-model-list">
            {state.models.map((model, index) => (
              <Button
                data-testid="setup.default-model-pick"
                data-item-id={model.alias}
                key={model.alias}
                type="button"
                variant={index === 0 ? "primary" : undefined}
                data-dialog-initial-focus={index === 0 ? "true" : undefined}
                disabled={state.saving}
                onClick={() => void controller.pick(model.alias)}
              >
                {model.label}
              </Button>
            ))}
          </div>
          {state.error === null ? null : (
            <p className="key-problem" role="alert">
              {state.error}
            </p>
          )}
        </section>
        <footer className="modal-actions">
          <Button data-testid="setup.default-model-later" type="button" onClick={later}>
            {TEXT.later}
          </Button>
        </footer>
      </div>
    </>
  );
}
