/**
 * "Add MCP server": the address (checked as you type with the same rule as the core,
 * `model/url`), an optional name, what Vibe will do, and a note for vendors that only allow approved clients.
 * "Add and sign in" asks the core, which shows the native confirmation, saves the server in Vibe's config and
 * starts the sign-in.  "This server needs an API key" switches to a key server (`KeySection`): the key
 * stays in the vault, and the core's native dialog asks whether to allow it until Vibe Work closes or always.
 * Depends on: useModalFocus, hooks, model/url, model/keyDraft, KeySection. Called by: ConnectorsPage.
 * Invariants: nothing is sent while the address or the key part is refused here; the core and the gate check
 * again; a typed key value is cleared when the dialog closes (it is unmounted).
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useCallback, useRef, useState } from "react";

import { useModalFocus } from "../../hooks/useModalFocus";
import type { KeyStatusView } from "../../protocol/core";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useKeyUi } from "../setup/useKeys";
import { useConnectorActions, useConnectors } from "./hooks";
import { KeySection } from "./KeySection";
import { emptyDraft, keyChoice } from "./model/keyDraft";
import { checkServerUrl } from "./model/url";

/** The dialog body (mounted only while open). */
function Dialog(): ReactElement {
  const actions = useConnectorActions();
  const prefill = useConnectors((s) => s.addPrefill);
  const dialog = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState(prefill?.url ?? "");
  const [name, setName] = useState(prefill?.name ?? "");
  const [needsKey, setNeedsKey] = useState(prefill?.key ?? false);
  const [keySectionCheck, setKeySectionCheck] = useState<{
    available: boolean;
    status: KeyStatusView | null;
  } | null>(null);
  const [draft, setDraft] = useState(() => emptyDraft(false));
  const [busy, setBusy] = useState(false);
  const vaultBlocked = useKeyReadiness("vault") !== null;
  const vaultStatus = useKeyUi((s) => s.status);
  const close = useCallback(() => actions?.showAdd(false), [actions]);
  useModalFocus(true, dialog, close);
  const text = url.trim();
  const check = text === "" ? null : checkServerUrl(text);
  const key = needsKey ? keyChoice(draft) : null;
  const keyReady = key === null || typeof key !== "string";
  const ok =
    check?.ok === true &&
    keyReady &&
    (!needsKey ||
      (!vaultBlocked &&
        keySectionCheck?.available === true &&
        keySectionCheck.status === vaultStatus)) &&
    !busy;
  const savedName = name.trim() === "" ? null : name.trim();
  const add = () => {
    setBusy(true);
    const done =
      key !== null && typeof key !== "string"
        ? actions?.addKeyServer(text, savedName, key)
        : actions?.addServer(text, savedName);
    void done?.finally(() => {
      setBusy(false);
    });
  };
  return (
    <>
      <div data-testid="connectors.add-server-dialog-scrim" className="scrim" onClick={close} />
      <div
        data-testid="connectors.add-server-dialog-modal"
        className="modal add-server"
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-server-title"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2 id="add-server-title">Add an MCP server</h2>
        </header>
        <section>
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
          {check?.ok === true && check.gated !== null && !needsKey ? (
            <p className="server-note" role="status" data-gated={check.gated.vendor}>
              <b>{check.gated.vendor} needs approval.</b> “{check.gated.rule}” ({check.gated.source}
              ). Mistral Vibe, the client Vibe signs in as, is not on that list yet, so the sign-in
              will probably be refused.
            </p>
          ) : null}
          <label className="cfield">
            <span>Name (optional)</span>
            <input
              data-testid="connectors.add-server-dialog-set-name"
              value={name}
              placeholder="Vibe picks one from the address"
              onChange={(e) => {
                setName(e.target.value);
              }}
            />
          </label>
        </section>
        <section>
          <label className="key-check">
            <input
              data-testid="connectors.add-server-dialog-set-needs-key"
              type="checkbox"
              checked={needsKey}
              onChange={(e) => {
                setNeedsKey(e.target.checked);
                if (e.target.checked) setKeySectionCheck(null);
              }}
            />{" "}
            This server needs an API key or token
          </label>
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
              <li>
                Vibe Work adds the key to each request to this server, and to no other address.
              </li>
              <li>Revoke it any time in Settings, Project secrets.</li>
            </ol>
          ) : (
            <ol className="csteps">
              <li>You confirm in a system dialog; Vibe saves the server in its config.</li>
              <li>Vibe asks the server how to sign in (OAuth) and registers as “Mistral Vibe”.</li>
              <li>The sign-in page opens in your browser after you check its address.</li>
              <li>
                The token stays in this computer's keychain (Vibe's); Vibe Work never sees it.
              </li>
            </ol>
          )}
          {typeof key === "string" ? <p className="cmuted">{key}</p> : null}
        </section>
        <footer className="modal-actions">
          <Button
            data-testid="connectors.add-server-dialog-tb-btn"
            type="button"

            onClick={close}
          >
            Cancel
          </Button>
          <Button
            data-testid="connectors.add-server-dialog-btn-primary"
            type="button"
            appearance="dialog"
            disabled={!ok}
            onClick={add}
          >
            {needsKey ? "Add with key" : "Add and sign in"}
          </Button>
        </footer>
      </div>
    </>
  );
}

/** Renders the dialog while it is open. */
export function AddServerDialog({ open }: { open: boolean }): ReactElement | null {
  return open ? <Dialog /> : null;
}
