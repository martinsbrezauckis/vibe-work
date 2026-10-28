/**
 * The explicit "trust this folder" action. Trust lets Vibe load the folder's hooks, agents and
 * plugins, i.e. run code that comes from the folder, so it is never implied by choosing or opening
 * a folder: it is its own button with the consequences spelled out, off by default. The gate only
 * accepts folders chosen with the native picker; for any other folder the action explains why.
 * Depends on: AppContext, hooks/useModalFocus. Called by: ProjectGroupView.
 * Invariants: nothing is sent until the user presses "Trust this folder"; Escape and Cancel send
 * nothing; the files Vibe reports (`detectedFiles`) are listed as text, never rendered.
 */
import { Button } from "../../components/Controls";
import { useEffect, useRef, useState } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { createStore, useStore } from "../../state/createStore";
import { useApp } from "../shell/AppContext";

const NO_GUIDANCE = createStore<string | null>(null);

/** The dialog for one folder. */
export function TrustDialog({ cwd, onClose }: { cwd: string; onClose: () => void }) {
  const { sessions } = useApp();
  const guidance = useStore(sessions?.trustGuidance ?? NO_GUIDANCE, (value) => value);
  const dialog = useRef<HTMLDivElement>(null);
  const [info, setInfo] = useState<{ status: string; files: string[] } | null>(null);
  useModalFocus(true, dialog, onClose);
  useEffect(() => {
    let live = true;
    sessions?.trustGuidance.set(null);
    void sessions?.trustStatus(cwd).then((answer) => {
      if (live) setInfo(answer);
    });
    return () => {
      live = false;
    };
  }, [sessions, cwd]);
  const trusted = info?.status === "trusted";
  return (
    <>
      <div data-testid="sessions.trust-dialog-scrim" className="scrim" onClick={onClose} />
      <div
        data-testid="sessions.trust-dialog-modal"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label="Trust this folder"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2>Trust this folder&apos;s hooks, agents and plugins?</h2>
        </header>
        <section>
          <p className="mono">{cwd}</p>
          <p>
            Trusting lets Vibe load and run code that lives in this folder: its hooks (commands that
            run on events), custom agents, tools and plugins. Only trust a folder whose contents you
            wrote or have read. Without trust, Vibe still works in the folder but ignores those
            files.
          </p>
          {info !== null && info.files.length > 0 && (
            <>
              <p>Vibe found these files that would load:</p>
              <ul className="mono">
                {info.files.slice(0, 12).map((f) => (
                  <li key={f}>{f}</li>
                ))}
              </ul>
            </>
          )}
          {trusted && <p>This folder is already trusted.</p>}
          {guidance !== null && (
            <p className="key-problem" role="alert">
              {guidance}
            </p>
          )}
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="sessions.trust-dialog-tb-btn"
            type="button"

            data-dialog-initial-focus="true"
            onClick={onClose}
          >
            Cancel
          </Button>
          <Button
            data-testid="sessions.trust-dialog-trust"
            type="button"
            appearance="dialog"
            disabled={trusted}
            onClick={() => {
              void sessions?.trust(cwd).then((ok) => {
                if (ok) onClose();
              });
            }}
          >
            Trust this folder
          </Button>
        </footer>
      </div>
    </>
  );
}
