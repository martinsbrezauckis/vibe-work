/** Settings Account's saved Mistral keys. Depends on the key controller and shared controls/focus.
 * Renders metadata only; every mutation goes through the typed window command client.
 */
import { useRef, useState } from "react";
import { Badge, Button, LinkButton } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import type { MistralKeyView } from "../../protocol/core";
import { chatsWorking, savedTestText } from "../../state/mistralKeys";
import { KeyStep } from "../setup/KeyStep";
import { PasswordAsk } from "../setup/PasswordAsk";
import { Problem } from "../setup/fields";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

const BUSY = "Wait until the chats finish, then switch keys.";

/** The Account list, immediately above Mistral's website links. */
export function MistralKeys() {
  const { keys } = useApp();
  const availability = useKeyUi((s) => s.availability);
  const status = useKeyUi((s) => s.status);
  const items = useKeyUi((s) => s.mistralKeys);
  const busy = useKeyUi((s) => s.busy);
  const switching = useKeyUi((s) => s.switching);
  const problem = useKeyUi((s) => s.problem);
  const working = useCoreState(chatsWorking);
  const ownProblem =
    problem?.operation !== undefined &&
    ["useMistralKey", "renameMistralKey", "removeMistralKey"].includes(problem.operation)
      ? problem.text
      : null;
  const [adding, setAdding] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [removing, setRemoving] = useState<MistralKeyView | null>(null);
  const root = useRef<HTMLElement>(null);
  const add = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocus(removing !== null, dialog, () => {
    if (!busy) setRemoving(null);
  });
  if (keys === null || availability !== "ready" || status === null) return null;
  const unlocked = status.vaultState === "unlocked";
  const rowFocus = (id: string | undefined) => {
    requestAnimationFrame(() => {
      const row = Array.from(
        root.current?.querySelectorAll<HTMLElement>("[data-mistral-key]") ?? [],
      ).find((el) => el.dataset["mistralKey"] === id);
      (row ?? add.current)?.focus();
    });
  };
  const cancelAdd = () => {
    setAdding(false);
    add.current?.focus();
  };

  return (
    <section ref={root} aria-labelledby="mistral-keys-title" data-setting="mistral-keys">
      <h3 id="mistral-keys-title">Mistral keys</h3>
      {!unlocked ? (
        <>
          <p className="key-line">Unlock your vault to see your Mistral keys.</p>
          <PasswordAsk
            action="Unlock"
            onSubmit={(password) => {
              void keys.unlock(password);
            }}
          />
        </>
      ) : (
        <>
          <div className="mistral-key-list">
            {items.map((item) => (
              <div
                key={item.itemId}
                data-testid="settings.mistral-keys-row"
                data-item-id={item.itemId}
                data-mistral-key={item.itemId}
                className="mistral-key-row"
                tabIndex={-1}
              >
                <div className="mistral-key-facts">
                  {renaming === item.itemId ? (
                    <form
                      data-testid="settings.mistral-keys-rename-form"
                      className="key-field"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void keys.renameMistralKey(item.itemId, name).then((ok) => {
                          if (ok) {
                            setRenaming(null);
                            rowFocus(item.itemId);
                          }
                        });
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Escape") {
                          event.stopPropagation();
                          setRenaming(null);
                          rowFocus(item.itemId);
                        }
                      }}
                    >
                      <label htmlFor="rename-mistral-key">Name</label>
                      <div className="key-field-row">
                        <input
                          data-testid="settings.mistral-keys-name"
                          id="rename-mistral-key"
                          value={name}
                          maxLength={200}
                          autoFocus
                          onChange={(event) => {
                            setName(event.target.value);
                          }}
                        />
                        <Button
                          data-testid="settings.mistral-keys-save-name"
                          shape="outlined"
                          type="submit"
                          disabled={busy || name.trim() === ""}
                        >
                          Save
                        </Button>
                        <Button
                          data-testid="settings.mistral-keys-cancel-name"
                          shape="outlined"
                          onClick={() => {
                            setRenaming(null);
                            rowFocus(item.itemId);
                          }}
                        >
                          Cancel
                        </Button>
                      </div>
                    </form>
                  ) : (
                    <span className="mistral-key-name" title={item.label}>
                      {item.label}
                    </span>
                  )}
                  <span className="mistral-key-test ui-caption">
                    {savedTestText(item.lastTest)}
                  </span>
                </div>
                {item.inUse && <Badge tone="ok">In use</Badge>}
                <div className="mistral-key-actions">
                  {!item.inUse && (
                    <Button
                      data-testid="settings.mistral-keys-use"
                      shape="outlined"
                      disabled={busy || working}
                      title={working ? BUSY : undefined}
                      onClick={() => {
                        void keys.useMistralKey(item.itemId).then(() => {
                          rowFocus(item.itemId);
                        });
                      }}
                    >
                      {switching === item.itemId ? "Switching…" : "Use this key"}
                    </Button>
                  )}
                  <LinkButton
                    data-testid="settings.mistral-keys-rename"
                    disabled={busy}
                    onClick={() => {
                      setName(item.label);
                      setRenaming(item.itemId);
                    }}
                  >
                    Rename
                  </LinkButton>
                  {!item.inUse && (
                    <Button
                      appearance="text-link"
                      variant="danger"
                      data-testid="settings.mistral-keys-remove"
                      disabled={busy}
                      onClick={() => {
                        setRemoving(item);
                      }}
                    >
                      Remove…
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
          {working && items.some((item) => !item.inUse) && (
            <p className="key-field-hint ui-help">{BUSY}</p>
          )}
          {!adding && (
            <Button
              data-testid="settings.mistral-keys-add"
              ref={add}
              shape="outlined"
              disabled={busy}
              onClick={() => {
                keys.clearProblem();
                setAdding(true);
              }}
            >
              Add key
            </Button>
          )}
          {adding && (
            <div
              data-testid="settings.mistral-keys-add-form"
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.stopPropagation();
                  cancelAdd();
                }
              }}
            >
              <KeyStep
                offers={false}
                additional
                onSaved={() => {
                  setAdding(false);
                  rowFocus(undefined);
                }}
              />
              <Button
                data-testid="settings.mistral-keys-cancel-add"
                shape="outlined"
                disabled={busy}
                onClick={cancelAdd}
              >
                Cancel
              </Button>
            </div>
          )}
          {!adding && <Problem text={ownProblem} />}
        </>
      )}
      {removing !== null && (
        <>
          <div
            className="scrim"
            data-testid="settings.mistral-keys-remove-scrim"
            onClick={() => {
              if (!busy) setRemoving(null);
            }}
          />
          <div
            data-testid="settings.mistral-keys-remove-dialog"
            className="modal"
            ref={dialog}
            role="dialog"
            aria-modal="true"
            aria-labelledby="mistral-key-remove-title"
            tabIndex={-1}
          >
            <header>
              <h2 id="mistral-key-remove-title">
                Remove the key “{removing.label}” from your vault?
              </h2>{" "}
            </header>
            <section>
              <p>Vibe Work can no longer use it. The key itself stays valid at Mistral.</p>
            </section>
            <footer className="modal-actions">
              <Button
                data-testid="settings.mistral-keys-cancel-remove"
                data-dialog-initial-focus="true"
                disabled={busy}
                onClick={() => {
                  setRemoving(null);
                }}
              >
                Cancel
              </Button>
              <Button
                data-testid="settings.mistral-keys-confirm-remove"
                variant="danger"
                disabled={busy}
                onClick={() => {
                  const next =
                    items[items.findIndex((item) => item.itemId === removing.itemId) + 1]?.itemId;
                  void keys.removeMistralKey(removing.itemId).then((ok) => {
                    if (ok) {
                      setRemoving(null);
                      rowFocus(next);
                    }
                  });
                }}
              >
                Remove
              </Button>
            </footer>
            <Problem text={ownProblem} />
          </div>
        </>
      )}
    </section>
  );
}
