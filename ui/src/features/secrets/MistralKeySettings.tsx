/**
 * The Vault page's model-key metadata, test outcome and one-way replacement. Depends on AppContext,
 * the existing key controller and shared Settings styles. Called by ProjectSecretsSettings.
 * The stored value is never read or copied; replacement input is cleared before the command
 * asks its native confirmation and writes the new key.
 */
import { type ReactNode, useId, useState } from "react";
import { TEXT } from "../../vocabulary";
import { Badge, Button, LinkButton, RowDisclosure } from "../../components/Controls";

import { when } from "../../state/secrets";
import { useKeyUi } from "../setup/useKeys";
import { useApp } from "../shell/AppContext";

/** The one metadata, test and replacement surface used by Account and Vault. */
export function MistralKeyDetails({
  location,
  extra,
}: {
  location: "account" | "vault";
  /** More actions on the same row (Account: Lock vault). */
  extra?: ReactNode;
}) {
  const { keys, shell } = useApp();
  const availability = useKeyUi((s) => s.availability);
  const status = useKeyUi((s) => s.status);
  const candidates = useKeyUi((s) => s.candidates);
  const test = useKeyUi((s) => s.test);
  const busy = useKeyUi((s) => s.busy);
  const problem = useKeyUi((s) => s.problem);
  const [replacing, setReplacing] = useState(false);
  const [value, setValue] = useState("");
  const inputId = useId();
  const ready = availability === "ready" && status?.keyState === "ready";
  const locked = status?.vaultState === "locked";
  const metadata = ready ? status.activeKey : null;
  const label = metadata?.label ?? candidates.find((c) => c.itemId === status?.keyItemId)?.label;
  const close = () => {
    setValue("");
    setReplacing(false);
    keys?.clearProblem();
  };
  return (
    <>
      {ready && location === "account" && test.phase !== "idle" && (
        <p className="key-line" role={test.phase === "failed" ? "alert" : "status"}>
          {test.phase === "running" && "Testing…"}
          {test.phase === "ok" && "Key works."}
          {test.phase === "failed" && test.text}
        </p>
      )}
      {ready && location === "vault" && (
        <dl className="set-facts">
          <div className="check">
            <dt>Key in use</dt>
            <dd>
              {label ?? (status.ownKey ? "Mistral API key (Vibe Work)" : "Selected Mistral key")}
            </dd>
          </div>
          <div className="check">
            <dt>Source</dt>
            <dd>
              {metadata?.source === "vibe" ? "Vibe’s key (saved in this vault)" : "This vault"}
            </dd>
          </div>
          <div className="check">
            <dt>Added</dt>
            <dd>{metadata == null ? "Unknown" : when(metadata.addedMs)}</dd>
          </div>
          <div className="check">
            <dt>Last tested</dt>
            <dd role={test.phase === "failed" ? "alert" : "status"}>
              {test.phase === "idle" && "Not tested this session."}
              {test.phase === "running" && "Testing…"}
              {test.phase === "ok" && "Key works."}
              {test.phase === "failed" && test.text}
            </dd>
          </div>
        </dl>
      )}
      {!ready && (
        <p className="key-line" role="status">
          {availability === "loading"
            ? "Reading key status…"
            : availability === "failed"
              ? "Could not read the Mistral key status. Use Retry to read it again."
              : availability === "none"
                ? "This run uses a key from its environment; vault key details are unavailable."
                : locked
                  ? "Unlock this vault to see the Mistral key in use."
                  : "No Mistral key is in use."}
        </p>
      )}
      <div className="set-actions" role="group" aria-label="Mistral key actions">
        {location === "vault" && (
          <>
            <LinkButton
              data-testid="secrets.mistral-key-settings-open-settings-2"
              onClick={() => {
                shell.actions.openSettings("account", "mistral-key");
              }}
            >
              Account
            </LinkButton>
            <LinkButton
              data-testid="secrets.mistral-key-settings-open-settings-3"
              onClick={() => {
                shell.actions.openSettings("usage", "session-usage");
              }}
            >
              Usage and plan
            </LinkButton>
          </>
        )}
        {ready && !replacing && (
          <>
            <Button
              data-testid="secrets.mistral-key-settings-test-key"
              shape="outlined"
              type="button"
              disabled={busy}
              onClick={() => {
                void keys?.testKey();
              }}
            >
              Test
            </Button>
            <Button
              data-testid="secrets.mistral-key-settings-clear-problem"
              shape="outlined"
              type="button"
              disabled={busy}
              onClick={() => {
                keys?.clearProblem();
                setReplacing(true);
              }}
            >
              Replace
            </Button>
          </>
        )}
        {extra}
        {location === "account" && (
          <LinkButton
            data-testid="secrets.mistral-key-settings-open-settings"
            onClick={() => {
              shell.actions.openSettings("secrets", "vault-mistral-key");
            }}
          >
            Manage in Vault
          </LinkButton>
        )}
        {!ready && availability === "ready" && (
          <Button
            data-testid="secrets.mistral-key-settings-open-settings-4"
            type="button"
            shape="outlined"
            onClick={() => {
              shell.actions.openSettings("account");
            }}
          >
            {locked ? "Unlock in Account" : "Set up the key"}
          </Button>
        )}
      </div>
      {ready && replacing && (
        <form
          className="key-offer"
          onSubmit={(event) => {
            event.preventDefault();
            if (keys === null || busy || value.trim() === "") return;
            const next = value;
            setValue("");
            // Start in this user event; the core owns confirmation before storage.
            void keys.saveKey(next).then((saved) => {
              if (saved) setReplacing(false);
            });
          }}
        >
          <div className="key-field">
            <label htmlFor={inputId}>New Mistral key</label>
            <div className="key-field-row">
              <input
                data-testid="secrets.mistral-key-settings-set-value"
                id={inputId}
                type="password"
                data-secret="true"
                value={value}
                autoFocus
                autoComplete="off"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                maxLength={4096}
                onChange={(event) => {
                  setValue(event.target.value);
                }}
              />
            </div>
          </div>
          {problem !== null && (
            <p className="key-problem" role="alert">
              {problem.text}
            </p>
          )}
          <div className="set-actions">
            <Button
              data-testid="secrets.mistral-key-settings-tb-btn"
              type="submit"
              shape="outlined"
              variant="primary"
              disabled={busy || value.trim() === ""}
            >
              Replace and test
            </Button>
            <Button
              data-testid="secrets.mistral-key-settings-tb-btn-2"
              type="button"
              shape="outlined"
              disabled={busy}
              onClick={close}
            >
              Cancel
            </Button>
          </div>
        </form>
      )}
    </>
  );
}

/** Metadata-only pinned Vault row; the Vault list owns its expansion state. */
export function MistralKeyRow({
  expanded,
  onExpandedChange,
}: {
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
}) {
  const detailId = useId();
  return (
    <div className="vault-model-key-row" data-setting="vault-mistral-key">
      <div className="check secrets-row">
        <RowDisclosure
          data-testid="secrets.mistral-key-settings-on-expanded-change"
          type="button"
          aria-label={`${expanded ? "Hide" : "Show"} details for Mistral key`}
          aria-expanded={expanded}
          aria-controls={expanded ? detailId : undefined}
          onClick={() => {
            onExpandedChange(!expanded);
          }}
        >
          <span className="secrets-name ui-body">Mistral key</span>
          <span className="ui-caption">{TEXT.model}</span>
        </RowDisclosure>
        <Badge title="Stored in your vault. Agents never see this value.">
          Agents never see it
        </Badge>
      </div>
      {expanded && (
        <div id={detailId} className="vault-entry-details">
          <MistralKeyDetails location="vault" />
        </div>
      )}
    </div>
  );
}
