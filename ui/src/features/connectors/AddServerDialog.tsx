/**
 * "Add MCP server": the address (checked as you type with the same rule as the core,
 * `model/url`), an optional name, what Vibe will do, and a note for vendors that only allow approved clients.
 * Four choices for any server: "No sign-in" (the core confirms, saves the server without an auth block and Vibe
 * connects at once), "API key" (a key server, `KeySection`: the key stays in the vault, and the core's native dialog
 * asks whether to allow it until Vibe Work closes or always), "Sign in" (the core confirms, saves the server
 * and starts the OAuth sign-in) and "Command on this computer" (a program Vibe starts; `CommandSection`). The last is
 * greyed with its reason and a button to Settings > Developer while Developer permissions is off. The choices are the
 * shared Selector: Tab reaches it, arrows, Home and End move, Enter picks.
 * Depends on: Modal, hooks, model/url, model/keyDraft, model/commandDraft, KeySection, CommandSection. Called by: ConnectorsPage.
 * Invariants: nothing is sent while the address or the key part is refused here; the core and the gate check
 * again; a typed key value is cleared when the dialog closes (it is unmounted). A refused add keeps the dialog open
 * and shows the core's reason inside it (the page's note is behind the dialog); opening the dialog clears the note.
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useCallback, useRef, useState } from "react";

import { Modal } from "../../components/Modal";
import { Selector } from "../../components/Selector";
import type { KeyStatusView } from "../../protocol/core";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useKeyUi } from "../setup/useKeys";
import { useConnectorActions, useConnectors } from "./hooks";
import { CommandSection } from "./CommandSection";
import { KeySection } from "./KeySection";
import {
  type CommandDraft,
  commandInput,
  commandProblem,
  emptyCommandDraft,
} from "./model/commandDraft";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { emptyDraft, keyChoice } from "./model/keyDraft";
import { checkServerUrl } from "./model/url";

type AuthMode = "none" | "key" | "oauth" | "command";

const AUTH_CHOICES: readonly { mode: AuthMode; label: string; hint: string }[] = [
  {
    mode: "none",
    label: "No sign-in",
    hint: "The server needs no key. It is open, or it runs here.",
  },
  { mode: "key", label: "API key", hint: "The server wants a key or token on each request." },
  { mode: "oauth", label: "Sign in", hint: "The server opens a sign-in page in your browser." },
];

/**
 * The auth choices in the shared pick-one Selector; each option says when it fits. "Command on this computer" is
 * greyed with its reason while Developer permissions is off, and a button next to it opens Settings > Developer.
 */
function AuthChoice({
  mode,
  developer,
  onChange,
  onDeveloperSettings,
}: {
  mode: AuthMode;
  developer: boolean;
  onChange: (mode: AuthMode) => void;
  onDeveloperSettings: () => void;
}): ReactElement {
  return (
    <fieldset className="cchoice">
      <legend>How does the server let you in?</legend>
      <Selector
        testId="connectors.add-server-dialog-auth"
        label="How does the server let you in?"
        value={mode}
        options={[
          ...AUTH_CHOICES.map((choice) => ({
            value: choice.mode,
            label: choice.label,
            description: choice.hint,
          })),
          {
            value: "command" as const,
            label: TEXT.commandServerChoice,
            description: developer
              ? TEXT.commandServerChoiceHint
              : TEXT.commandServerNeedsDeveloper,
            disabled: !developer,
          },
        ]}
        onChange={onChange}
      />
      {developer ? null : (
        <p className="cmuted" role="status">
          {TEXT.commandServerChoice}: {TEXT.commandServerNeedsDeveloper}.{" "}
          <Button
            data-testid="connectors.add-server-dialog-developer-settings"
            type="button"
            shape="outlined"
            onClick={onDeveloperSettings}
          >
            {TEXT.commandServerDeveloperButton}
          </Button>
        </p>
      )}
    </fieldset>
  );
}

/** The dialog body (mounted only while open). */
function Dialog(): ReactElement {
  const actions = useConnectorActions();
  const prefill = useConnectors((s) => s.addPrefill);
  const dialog = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState(prefill?.url ?? "");
  const [name, setName] = useState(prefill?.name ?? "");
  const [mode, setMode] = useState<AuthMode>(prefill?.key === true ? "key" : "oauth");
  const needsKey = mode === "key";
  const command = mode === "command";
  const developer = useConnectors((s) => s.view?.developerPermissions === true);
  const { shell } = useApp();
  const [commandDraft, setCommandDraft] = useState<CommandDraft>(emptyCommandDraft);
  const [keySectionCheck, setKeySectionCheck] = useState<{
    available: boolean;
    status: KeyStatusView | null;
  } | null>(null);
  const [draft, setDraft] = useState(() => emptyDraft(false));
  const [busy, setBusy] = useState(false);
  const note = useConnectors((s) => s.note);
  const vaultBlocked = useKeyReadiness("vault") !== null;
  const vaultStatus = useKeyUi((s) => s.status);
  const close = useCallback(() => actions?.showAdd(false), [actions]);
  const text = url.trim();
  const check = text === "" ? null : checkServerUrl(text);
  const key = needsKey ? keyChoice(draft) : null;
  const keyReady = key === null || typeof key !== "string";
  const commandIssue = command ? commandProblem(commandDraft) : null;
  const ok = command
    ? developer && commandIssue === null && !busy
    : check?.ok === true &&
      keyReady &&
      (!needsKey ||
        (!vaultBlocked &&
          keySectionCheck?.available === true &&
          keySectionCheck.status === vaultStatus)) &&
      !busy;
  /** What Add still needs, in the words of the current mode; null once nothing is missing (or while it is busy). */
  const missing = busy
    ? null
    : command
      ? commandDraft.command.trim() === ""
        ? "Type the program to run."
        : null
      : check === null
        ? "Enter the server's address to continue."
        : null;
  const savedName = name.trim() === "" ? null : name.trim();
  const add = () => {
    setBusy(true);
    const done = command
      ? actions?.addCommandServer(commandInput({ ...commandDraft, name }, true))
      : key !== null && typeof key !== "string"
        ? actions?.addKeyServer(text, savedName, key)
        : mode === "none"
          ? actions?.addNoAuthServer(text, savedName)
          : actions?.addServer(text, savedName);
    void done?.finally(() => {
      setBusy(false);
    });
  };
  return (
    <Modal
      onClose={close}
      scrimTestId="connectors.add-server-dialog-scrim"
      dialogRef={dialog}
      data-testid="connectors.add-server-dialog-modal"
      className="modal add-server"
      aria-labelledby="add-server-title"
    >
      <header>
        <h2 id="add-server-title">Add an MCP server</h2>
      </header>
      <section>
        {command ? null : (
          <>
            <label className="cfield">
              <span>Address</span>
              <input
                data-testid="connectors.add-server-dialog-set-url"
                data-dialog-initial-focus="true"
                value={url}
                placeholder="https://example.com/mcp"
                spellCheck={false}
                autoComplete="off"
                aria-invalid={check !== null && !check.ok}
                aria-describedby="add-server-check"
                onChange={(e) => {
                  setUrl(e.target.value);
                }}
              />
            </label>
            <p
              role={check !== null && !check.ok ? "alert" : "status"}
              id="add-server-check"
              className={check !== null && !check.ok ? "key-problem" : "cmuted"}
            >
              {check === null
                ? "https:// for servers on the internet; http:// only for a server on this computer."
                : check.ok
                  ? `Goes to ${check.host}${check.local ? " (this computer)" : ""}.`
                  : check.reason}
            </p>
            {check?.ok === true && check.gated !== null && mode === "oauth" ? (
              <p className="server-note" role="status" data-gated={check.gated.vendor}>
                <b>{check.gated.vendor} needs approval.</b> “{check.gated.rule}” (
                {check.gated.source}
                ). Mistral Vibe, the client Vibe signs in as, is not on that list yet, so the
                sign-in will probably be refused.
              </p>
            ) : null}
          </>
        )}
        <label className="cfield">
          <span>Name (optional)</span>
          <input
            data-testid="connectors.add-server-dialog-set-name"
            value={name}
            placeholder={
              command ? "Taken from the program's name" : "Vibe picks one from the address"
            }
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </label>
      </section>
      <section>
        <AuthChoice
          mode={mode}
          developer={developer}
          onChange={(next) => {
            setMode(next);
            if (next === "key") setKeySectionCheck(null);
          }}
          onDeveloperSettings={() => {
            close();
            shell.actions.openSettings("developer");
          }}
        />
        {command ? <CommandSection draft={commandDraft} onChange={setCommandDraft} /> : null}
        {needsKey ? (
          <KeySection
            draft={draft}
            onChange={setDraft}
            onAvailableChange={(available, status) => {
              setKeySectionCheck({ available, status });
            }}
          />
        ) : null}
      </section>
      <section>
        <h3>What happens</h3>
        {needsKey ? (
          <ol className="csteps">
            <li>
              You choose in a system dialog: always (recommended), until Vibe Work closes, or not.
            </li>
            <li>The key stays in your vault. Vibe gets only an address on this computer.</li>
            <li>Vibe Work adds the key to each request to this server, and to no other address.</li>
            <li>Revoke it any time in Settings, Project secrets.</li>
          </ol>
        ) : command ? (
          <ol className="csteps">
            <li>
              You confirm in a system dialog. It shows the program Vibe Work found (its full path),
              each argument, the working folder and the variable names.
            </li>
            <li>Vibe saves the server and starts the program when a chat connects to it.</li>
            <li>
              The program runs on this computer with your account's rights. Vibe Work does not
              inspect or vouch for it.
            </li>
            <li>
              Variable values are saved as plain text in Vibe's settings file; keep keys out of
              them.
            </li>
          </ol>
        ) : mode === "none" ? (
          <ol className="csteps">
            <li>
              You confirm in a system dialog. It says where the server runs and what it can reach.
            </li>
            <li>Vibe saves the server and connects at once, with no key and no sign-in.</li>
            <li>
              Vibe Work does not inspect or vouch for the server: it sees what your chats send it.
            </li>
          </ol>
        ) : (
          <ol className="csteps">
            <li>You confirm in a system dialog; Vibe saves the server in its config.</li>
            <li>Vibe asks the server how to sign in (OAuth) and registers as “Mistral Vibe”.</li>
            <li>The sign-in page opens in your browser after you check its address.</li>
            <li>The token stays in this computer's keychain (Vibe's); Vibe Work never sees it.</li>
          </ol>
        )}
        {typeof key === "string" ? <p className="cmuted">{key}</p> : null}
        {commandIssue !== null && commandDraft.command.trim() !== "" ? (
          <p className="key-problem" role="alert">
            {commandIssue}
          </p>
        ) : null}
        {note === null ? null : (
          <p className="key-problem" role="alert">
            {note}
          </p>
        )}
      </section>
      <footer className="modal-actions">
        <Button
          data-testid="connectors.add-server-dialog-tb-btn"
          type="button"

          onClick={close}
        >
          Cancel
        </Button>
        {missing !== null ? (
          <span id="add-server-missing" className="cfield-reason">
            {missing}
          </span>
        ) : null}
        <Button
          data-testid="connectors.add-server-dialog-btn-primary"
          type="button"
          variant="primary"
          disabled={!ok}
          {...(missing !== null ? { "aria-describedby": "add-server-missing" } : {})}
          onClick={add}
        >
          {command
            ? "Add command server"
            : mode === "key"
              ? "Add with key"
              : mode === "none"
                ? "Add server"
                : "Add and sign in"}
        </Button>
      </footer>
    </Modal>
  );
}

/** Renders the dialog while it is open. */
export function AddServerDialog({ open }: { open: boolean }): ReactElement | null {
  return open ? <Dialog /> : null;
}
