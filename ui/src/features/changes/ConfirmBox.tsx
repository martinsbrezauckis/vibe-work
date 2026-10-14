/**
 * The Changes tab's confirmations, each naming exactly what will happen: files a revert deletes, what an
 * undo restores and cannot restore (and that later turns go too), what `git init` + a first commit adds.
 * Depends on: store.ts (pending confirmation, accept/cancel), hooks/useModalFocus. Called by: ChangesTab.
 * Invariants: nothing is sent until the confirm button is pressed; Escape and Cancel send nothing; file
 * names are text, never rendered markup.
 */
import { type ReactElement, type ReactNode, useRef } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import { useStore } from "../../state/createStore";
import { fileNote } from "./model";
import type { Confirm } from "./store";
import type { Changes } from "./store";

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
              The conversation goes back to before turn {confirm.turnNumber}
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
        action: "Run git init and commit",
        body: (
          <>
            <p>
              Runs <code>git init</code> in <span className="mono">{confirm.plan.folder}</span> and
              commits every file in it as a first commit ({confirm.plan.files} files,{" "}
              {size(confirm.plan.bytes)}).
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
            {confirm.plan.fallbackAuthor !== null && (
              <p>
                Git has no name set up, so the commit is authored as {confirm.plan.fallbackAuthor}.
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
        className="scrim"
        onClick={() => {
          changes.cancel();
        }}
      />
      <div
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
        <section>{body}</section>
        <footer className="modal-actions">
          <button
            type="button"
            className="tb-btn"
            data-dialog-initial-focus="true"
            onClick={() => {
              changes.cancel();
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn-primary"
            data-action="confirm"
            onClick={() => void changes.accept()}
          >
            {action}
          </button>
        </footer>
      </div>
    </>
  );
}
