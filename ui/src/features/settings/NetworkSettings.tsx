/**
 * Settings > Network: the proxy and the company certificate Vibe Work and Vibe use to reach Mistral on a company
 * network (Vibe's `/proxy-setup`). Proxy addresses (HTTPS, HTTP, anything else), the addresses that skip the proxy,
 * the certificate file with a native picker, a connection test and Save.
 * Depends on: useNetwork (state and actions), components/Controls (`Button`), shared Settings pieces (`key-field`,
 * `key-field-row`, `tb-btn` through `Button`, `ui-help`, `key-problem`, `key-success`). Called by: settings/SettingsPage.
 * Invariants:
 * - An address that has a sign-in is shown masked (`http://***@host`) with a short state; typing a new address
 *   replaces it. The proxy inputs are marked `data-secret` (a typed sign-in must not appear in a screenshot).
 * - Every saved change says plainly when it applies (the next start of Vibe Work); the screen never claims a setting
 *   is "active" that the running app has not loaded (`restartNeeded` comes from the core).
 * - A value the core refuses shows its sentence under its field; the test names the failure and the next step.
 * - Nothing is saved without the native confirmation the core asks on Save.
 */
import { literalInputProps } from "../../components/inputProps";
import { useId, useState, type ReactElement, type ReactNode } from "react";

import { Button } from "../../components/Controls";
import type { NetworkFieldView } from "../../protocol/core";
import { useNetwork } from "./useNetwork";

/** One text field with its label and current state. */
function Field(props: {
  testId: string;
  label: string;
  value: string;
  placeholder: string;
  field: NetworkFieldView | null;
  problem: string | undefined;
  disabled: boolean;
  secret?: boolean;
  onChange: (text: string) => void;
  extra?: ReactNode;
}): ReactElement {
  const id = useId();
  const [editing, setEditing] = useState(false);
  const { field } = props;
  const mask = props.secret === true && (editing || props.value !== (field?.value ?? ""));
  return (
    <div className="key-field">
      <label htmlFor={id}>{props.label}</label>
      <div className="key-field-row">
        <input
          {...literalInputProps}
          data-testid={props.testId}
          id={id}
          type={mask ? "password" : "text"}
          {...(props.secret === true ? { "data-secret": "true" } : {})}
          value={props.value}
          placeholder={props.placeholder}
          disabled={props.disabled}
          autoComplete="off"
          onFocus={() => {
            setEditing(true);
          }}
          onBlur={() => {
            setEditing(false);
          }}
          aria-invalid={props.problem === undefined ? undefined : true}
          onChange={(e) => {
            props.onChange(e.target.value);
          }}
        />
        {props.extra}
      </div>
      {field?.hasSignIn === true && props.value === field.value && (
        <p className="key-field-hint ui-help">Sign-in hidden</p>
      )}
      {field?.environmentValue != null && (
        <p className="key-field-hint ui-help" data-level="short">
          From environment: {field.environmentValue}
        </p>
      )}
      {field?.problem != null && (
        <p className="key-field-hint ui-help" data-level="short">
          Saved value: {field.problem}
        </p>
      )}
      {props.problem !== undefined && (
        <p className="key-problem" role="alert">
          {props.problem}
        </p>
      )}
    </div>
  );
}

/** The section. */
export function NetworkSettings(): ReactElement | null {
  const n = useNetwork();
  if (n.core === null) return null;
  const locked = n.busy !== null || n.view === null;
  const view = n.view;
  return (
    <section className="net-section" aria-labelledby="network-title">
      <h3 id="network-title">Proxy and certificate</h3>
      {!n.running && (
        <p className="key-line" role="status">
          Vibe is not running
        </p>
      )}
      {n.error !== null && (
        <div className="key-offer">
          <p className="key-problem" role="alert">
            {n.error}
          </p>
          <div className="set-actions">
            <Button
              data-testid="settings.network-settings-read"
              shape="outlined"
              disabled={n.busy !== null}
              onClick={n.read}
            >
              Try again
            </Button>
          </div>
        </div>
      )}
      {view === null && n.error === null && <p className="key-line">Reading the settings…</p>}
      {view !== null && (
        <>
          {view.restartNeeded && n.note === null && (
            <p className="key-line" role="status">
              Restart required
            </p>
          )}
          {n.note !== null && (
            <p className="key-success" role="status">
              {n.note}
            </p>
          )}
          <div data-setting="network-proxy">
            <Field
              testId="settings.network-settings-set-https-proxy"
              label="HTTPS proxy"

              value={n.form.httpsProxy}
              placeholder="http://proxy.example.com:8080"
              field={view.httpsProxy}
              problem={n.problems.httpsProxy}
              disabled={locked}
              secret
              onChange={(text) => {
                n.edit("httpsProxy", text);
              }}
            />
            <Field
              testId="settings.network-settings-set-http-proxy"
              label="HTTP proxy"

              value={n.form.httpProxy}
              placeholder="http://proxy.example.com:8080"
              field={view.httpProxy}
              problem={n.problems.httpProxy}
              disabled={locked}
              secret
              onChange={(text) => {
                n.edit("httpProxy", text);
              }}
            />
            <Field
              testId="settings.network-settings-set-all-proxy"
              label="Proxy for anything else (optional)"

              value={n.form.allProxy}
              placeholder="http://proxy.example.com:8080"
              field={view.allProxy}
              problem={n.problems.allProxy}
              disabled={locked}
              secret
              onChange={(text) => {
                n.edit("allProxy", text);
              }}
            />
          </div>
          <div data-setting="network-skip">
            <Field
              testId="settings.network-settings-set-no-proxy"
              label="Addresses that skip the proxy (optional)"
              value={n.form.noProxy}
              placeholder=".intranet.example.com, 10.0.0.0/8"
              field={view.noProxy}
              problem={n.problems.noProxy}
              disabled={locked}
              onChange={(text) => {
                n.edit("noProxy", text);
              }}
            />
          </div>
          <div data-setting="network-certificate">
            <Field
              testId="settings.network-settings-set-certificate-file"
              label="Company certificate file"

              value={n.form.certificateFile}
              placeholder="No certificate file chosen"
              field={view.certificateFile}
              problem={n.problems.certificateFile}
              disabled={locked}
              onChange={(text) => {
                n.edit("certificateFile", text);
              }}
              extra={
                <>
                  <Button
                    data-testid="settings.network-settings-choose-certificate"
                    shape="outlined"
                    disabled={locked}
                    onClick={n.chooseCertificate}
                  >
                    Choose file…
                  </Button>
                  {n.form.certificateFile !== "" && (
                    <Button
                      data-testid="settings.network-settings-clear-certificate"
                      shape="outlined"
                      disabled={locked}
                      onClick={() => {
                        n.edit("certificateFile", "");
                      }}
                    >
                      Remove
                    </Button>
                  )}
                </>
              }
            />
            {view.certificateCount !== null &&
              n.form.certificateFile === view.certificateFile.value && (
                <p className="key-field-hint ui-help">
                  {view.certificateCount === 1
                    ? "1 certificate"
                    : `${String(view.certificateCount)} certificates`}
                </p>
              )}
            {(view.certificateFolder.value !== "" || n.form.certificateFolder !== "") && (
              <Field
                testId="settings.network-settings-set-certificate-folder"
                label="Company certificate folder"

                value={n.form.certificateFolder}
                placeholder=""
                field={view.certificateFolder}
                problem={n.problems.certificateFolder}
                disabled={locked}
                onChange={(text) => {
                  n.edit("certificateFolder", text);
                }}
              />
            )}
          </div>
          {n.busy === "test" && (
            <p className="key-line" role="status">
              Testing the connection to Mistral…
            </p>
          )}
          {n.outcome !== null && (
            <div className={n.outcome.reached ? "key-success" : "key-problem"} role="status">
              <p>{n.outcome.summary}</p>
              {n.outcome.nextStep !== null && <p className="ui-help">{n.outcome.nextStep}</p>}
              {n.outcome.overriddenByEnvironment.length > 0 && (
                <p className="ui-help">
                  From environment: {n.outcome.overriddenByEnvironment.join(", ")}.
                </p>
              )}
            </div>
          )}
          <div className="set-actions">
            <Button
              data-testid="settings.network-settings-test-connection"
              shape="outlined"
              disabled={locked}
              onClick={n.testConnection}
            >
              Test connection
            </Button>
            <Button
              data-testid="settings.network-settings-save"
              shape="outlined"
              variant="primary"
              disabled={locked || !n.dirty}
              onClick={n.save}
            >
              Save
            </Button>
          </div>
        </>
      )}
    </section>
  );
}
