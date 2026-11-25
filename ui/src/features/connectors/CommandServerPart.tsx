/**
 * A command-started server's part of the details panel: what starts it (the program, arguments, working folder,
 * variables; a value that looks like a key is shown as hidden), "Edit command", and, while Developer permissions is
 * off, the reason and a button that opens Settings > Developer. Changing the command asks the same native dialog as
 * adding it; the program is resolved again by the core.
 * Depends on: AppContext, hooks, CommandSection, model/commandDraft, Modal. Called by: ConnectorDetail.
 * Invariants: Edit is greyed with its reason while Developer permissions is off (never a dead click); a hidden
 * variable is sent back as "keep as saved"; the dialog stays open with the core's reason when a change is refused.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useRef, useState } from "react";

import { Modal } from "../../components/Modal";
import type { ServerView } from "../../protocol/core";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { CommandSection } from "./CommandSection";
import { useConnectorActions, useConnectors } from "./hooks";
import { commandInput, commandProblem, draftOf } from "./model/commandDraft";

function EditDialog({
  server,
  onClose,
}: {
  server: ServerView;
  onClose: () => void;
}): ReactElement {
  const actions = useConnectorActions();
  const dialog = useRef<HTMLDivElement>(null);
  const note = useConnectors((s) => s.note);
  const [draft, setDraft] = useState(() =>
    draftOf(server.name, server.commandConfig ?? { command: "", args: [], env: [] }),
  );
  const [busy, setBusy] = useState(false);
  const problem = commandProblem(draft);
  return (
    <Modal
      onClose={onClose}
      scrimTestId="connectors.edit-command-dialog-scrim"
      dialogRef={dialog}
      data-testid="connectors.edit-command-dialog-modal"
      className="modal add-server"
      aria-labelledby="edit-command-title"
    >
      <header>
        <h2 id="edit-command-title">Edit command</h2>
      </header>
      <section>
        <CommandSection draft={draft} onChange={setDraft} />
        <p className="cmuted">
          You confirm the change in a system dialog. The program runs on this computer with your
          account's rights; Vibe Work does not inspect or vouch for it.
        </p>
        {problem !== null && draft.command.trim() !== "" ? (
          <p className="key-problem" role="alert">
            {problem}
          </p>
        ) : null}
        {note === null ? null : (
          <p className="key-problem" role="alert">
            {note}
          </p>
        )}
      </section>
      <footer className="modal-actions">
        <Button data-testid="connectors.edit-command-dialog-cancel" type="button" onClick={onClose}>
          Cancel
        </Button>
        <Button
          data-testid="connectors.edit-command-dialog-save"
          type="button"
          variant="primary"
          disabled={problem !== null || busy}
          onClick={() => {
            setBusy(true);
            void actions
              ?.updateCommandServer(server.name, commandInput(draft, false))
              .then((saved) => {
                if (saved) onClose();
              })
              .finally(() => {
                setBusy(false);
              });
          }}
        >
          Save command
        </Button>
      </footer>
    </Modal>
  );
}

/** The command summary and actions of a configured command server. */
export function CommandServerPart({ server }: { server: ServerView }): ReactElement {
  const { shell } = useApp();
  const developer = useConnectors((s) => s.view?.developerPermissions === true);
  const [editing, setEditing] = useState(false);
  const config = server.commandConfig;
  const paused = server.errorKind === "developerPermissionsOff";
  return (
    <>
      <h3>What starts it</h3>
      {config === undefined ? null : (
        <dl className="cmeta" data-command-server={server.name}>
          <dt>Program</dt>
          <dd className="mono">{config.command}</dd>
          <dt>Arguments</dt>
          <dd className="mono">{config.args.length === 0 ? "none" : config.args.join(" ")}</dd>
          <dt>Working folder</dt>
          <dd className="mono">{config.cwd ?? "Vibe's own folder"}</dd>
          <dt>Variables</dt>
          <dd className="mono">
            {config.env.length === 0
              ? "none"
              : config.env
                  .map((e) => `${e.name}=${e.hidden ? "[hidden]" : (e.value ?? "")}`)
                  .join(" ")}
          </dd>
        </dl>
      )}
      {developer || paused ? null : <p className="cmuted">{TEXT.commandServerNeedsDeveloper}.</p>}
      <div className="cactions">
        <Button
          data-testid="connectors.connector-detail-edit-command"
          type="button"
          shape="outlined"
          disabled={!developer}
          title={developer ? undefined : TEXT.commandServerNeedsDeveloper}
          onClick={() => {
            setEditing(true);
          }}
        >
          Edit command
        </Button>
        {developer ? null : (
          <Button
            data-testid="connectors.connector-detail-developer-settings"
            type="button"
            shape="outlined"
            onClick={() => {
              shell.actions.openSettings("developer");
            }}
          >
            {TEXT.commandServerDeveloperButton}
          </Button>
        )}
      </div>
      {editing ? (
        <EditDialog
          server={server}
          onClose={() => {
            setEditing(false);
          }}
        />
      ) : null}
    </>
  );
}
