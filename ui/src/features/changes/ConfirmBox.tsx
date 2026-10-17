/**
 * The Changes tab's confirmations, each naming exactly what will happen: files a revert deletes, what an
 * undo restores and cannot restore (and that later turns go too), what `git init` + a first commit adds.
 * Depends on: store.ts (pending confirmation, accept/cancel), hooks/useModalFocus. Called by: ChangesTab.
 * Invariants: nothing is sent until the confirm button is pressed; Escape and Cancel send nothing; file
 * names are text, never rendered markup.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, type ReactNode, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { fileNote } from "./model";
import type { Confirm } from "./store";
import type { Changes } from "./store";
import { GitIdentityFields } from "../git/GitIdentityFields";
import { validIdentity } from "../../state/gitIdentity";

const SHOWN = 20;

function Paths({ paths }: { paths: readonly string[] }): ReactElement {
  return (
    <ul className="mono">
      {paths.slice(0, SHOWN).map((p) => (
        <li key={p}>{p}</li>
      ))}
      {paths.length > SHOWN && <li>and {String(paths.length - SHOWN)} more</li>}
    </ul>
  );
}

function size(bytes: number): string {
  if (bytes === 0) return "0 KB";
  return bytes >= 1024 * 1024
    ? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    : `${String(Math.max(1, Math.round(bytes / 1024)))} KB`;
}

function content(confirm: Confirm): { title: string; action: string; body: ReactNode } {
  switch (confirm.kind) {
    case "keepUnseen":
      return {
        title: `Keep ${confirm.file.path} without seeing it?`,
        action: "Keep without seeing it",
        body: (
          <>
            <p>
              This change is not shown line by line ({fileNote(confirm.file)}). Keeping it accepts
              the whole file as it is now.
            </p>
            <Paths paths={[confirm.file.path]} />
          </>
        ),
      };
    case "revertFile":
      return {
        title: `Delete ${confirm.file.path}?`,
        action: "Revert and delete",
        body: (
          <>
            <p>
              This file did not exist when the session started. Reverting it deletes it from the
              folder:
            </p>
            <Paths paths={confirm.deletes} />
          </>
        ),
      };
    case "revertAll":
      return {
        title: `Revert all ${String(confirm.files)} changed files?`,
        action: confirm.deletes.length > 0 ? "Revert all and delete" : "Revert all",
        body: (
          <>
            <p>Every file goes back to how it was when the session started.</p>
            {confirm.deletes.length > 0 && (
              <>
                <p>These files were created in the session and will be deleted:</p>
                <Paths paths={confirm.deletes} />
              </>
            )}
          </>
        ),
      };
    case "undoTurn": {
      const p = confirm.preview;
      return {
        title: `Undo turn ${String(confirm.turnNumber)}?`,
        action: "Undo turn",
        body: (
          <>
            <p>
              The session goes back to before turn {confirm.turnNumber}
              {p.turnsRemoved > 1 ? ` (${String(p.turnsRemoved)} turns are removed)` : ""}, and your
              message is put back in the box.
            </p>
            {p.restorable.length > 0 ? (
              <>
                <p>Vibe puts these files back:</p>
                <Paths paths={p.restorable} />
              </>
            ) : (
              <p>Vibe has no saved copies to put back.</p>
            )}
            {p.notRestorable.length > 0 && (
              <>
                <p>These stay as they are now (Vibe saved no copy from before):</p>
                <Paths paths={p.notRestorable} />
              </>
            )}
          </>
        ),
      };
    }
    case "history":
      return {
        title: "Turn on version history?",
        action: "Turn on version history",
        body: (
          <>
            <p>
              Vibe Work starts keeping versions of{" "}
              <span className="mono">{confirm.plan.folder}</span> and saves a first version of{" "}
              {confirm.plan.files === 1 ? "its" : "all"} {confirm.plan.files}{" "}
              {confirm.plan.files === 1 ? "file" : "files"} ({size(confirm.plan.bytes)}). This uses
              git; nothing leaves this computer.
            </p>
            {confirm.plan.generated.length > 0 && (
              <>
                <p>These generated folders are included (there is no .gitignore for them):</p>
                <Paths paths={confirm.plan.generated} />
              </>
            )}
            {confirm.plan.large.length > 0 && (
              <>
                <p>These large files are left out:</p>
                <Paths paths={confirm.plan.large} />
              </>
            )}
            {!confirm.needsIdentity && (
              <p>
                Commit author: {confirm.identity.name} &lt;{confirm.identity.email}&gt;. These
                details are saved locally in the new repository.
              </p>
            )}
          </>
        ),
      };
  }
}

/** The pending confirmation, if any. */
export function ConfirmBox({ changes }: { changes: Changes }): ReactElement | null {
  const confirm = useStore(changes.store, (s) => s.confirm);
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocus(confirm !== null, dialog, () => {
    changes.cancel();
  });
  if (confirm === null) return null;
  const { title, action, body } = content(confirm);
  return (
    <>
      <div
        data-testid="changes.confirm-box-cancel"
        className="scrim"
        onClick={() => {
          changes.cancel();
        }}
      />
      <div
        data-testid="changes.confirm-box-modal"
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        ref={dialog}
        tabIndex={-1}
        data-confirm={confirm.kind}
      >
        <header>
          <h2>{title}</h2>
        </header>
        <section>
          {body}
          {confirm.kind === "history" && confirm.needsIdentity && (
            <>
              <p>
                Enter your name and email for commits. Saved in Vibe Work for future repositories
                and locally in this new repository.
              </p>
              <GitIdentityFields
                identity={confirm.identity}
                onChange={(identity) => {
                  changes.editHistoryIdentity(identity);
                }}
              />
            </>
          )}
        </section>
        <footer className="modal-actions">
          <button
            data-testid="changes.confirm-box-cancel-2"
            type="button"
            className="tb-btn"
            data-dialog-initial-focus="true"
            onClick={() => {
              changes.cancel();
            }}
          >
            Cancel
          </button>
          {confirm.kind !== "keepUnseen" && confirm.kind !== "history" ? (
            <Button
              data-testid="changes.confirm-box-accept"
              small
              variant="danger"
              data-action="confirm"
              onClick={() => void changes.accept()}
            >
              {action}
            </Button>
          ) : (
            <button
              data-testid="changes.confirm-box-accept"
              type="button"
              className="btn-primary"
              data-action="confirm"
              disabled={confirm.kind === "history" && !validIdentity(confirm.identity)}
              onClick={() => void changes.accept()}
            >
              {action}
            </button>
          )}
        </footer>
      </div>
    </>
  );
}
