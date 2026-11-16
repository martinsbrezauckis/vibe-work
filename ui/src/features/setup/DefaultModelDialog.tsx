/**
 * The card that asks for a new default model when Vibe no longer offers the Settings default: the gone model's name,
 * one button per model Vibe offers, and Later.
 * Depends on: AppContext (the default-model controller), createStore, Modal, Controls, vocabulary.
 * Called by: Shell.
 * Invariants: shown exactly while the controller names a gone default; modal (`aria-modal`), Escape and the scrim are
 * Later; nothing is written until a model is picked.
 */
import type { ReactElement } from "react";

import { Button } from "../../components/Controls";
import { Modal } from "../../components/Modal";
import { createStore, useStore } from "../../state/createStore";
import type { DefaultModelState } from "../../state/defaultModel";
import { TEXT, wording } from "../../vocabulary";
import { useApp } from "../shell/AppContext";

const NONE = createStore<DefaultModelState>({ gone: null, models: [], saving: false, error: null });

/** The card, or nothing. */
export function DefaultModelDialog(): ReactElement | null {
  const controller = useApp().defaultModel;
  const state = useStore(controller?.store ?? NONE, (s) => s);
  const later = () => {
    controller?.later();
  };
  if (controller === null || state.gone === null) return null;
  return (
    <Modal
      onClose={later}
      scrimTestId="setup.default-model-scrim"
      className="modal default-model-dialog"
      aria-labelledby="default-model-title"
      data-testid="setup.default-model-dialog"
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
    </Modal>
  );
}
