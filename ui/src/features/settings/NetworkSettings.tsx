/**
 * Settings > Network: the proxy and the company certificate Vibe Work and Vibe use to reach Mistral on a company
 * network (Vibe's `/proxy-setup`). Proxy addresses (HTTPS, HTTP, anything else), the addresses that skip the proxy,
 * the certificate file with a native picker, a connection test and Save.
 * Depends on: useNetwork (state and actions), components/Controls (`Button`), shared Settings pieces (`key-field`,
 * `key-field-row`, `tb-btn` through `Button`, `ui-help`, `key-problem`, `key-success`). Called by: settings/SettingsPage.
 * Invariants:
 * - An address that has a sign-in is shown masked (`http://***@host`) with a line saying so; typing a new address
 *   replaces it. The proxy inputs are marked `data-secret` (a typed sign-in must not appear in a screenshot).
 * - Every saved change says plainly when it applies (the next start of Vibe Work); the screen never claims a setting
 *   is "active" that the running app has not loaded (`restartNeeded` comes from the core).
 * - A value the core refuses shows its sentence under its field; the test names the failure and the next step.
 * - Nothing is saved without the native confirmation the core asks on Save.
 */
import { useId, type ReactElement, type ReactNode } from "react";

import { Button } from "../../components/Controls";
import type { NetworkFieldView } from "../../protocol/core";
import { useNetwork } from "./useNetwork";

/** One text field with its label, helper line and the sentences that belong to it. */
function Field(props: {
  testId: string;
  label: string;
  hint: ReactNode;
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
  const { field } = props;
  return (
    <div className="key-field">
      <label htmlFor={id}>{props.label}</label>
      <div className="key-field-row">
        <input
          data-testid={props.testId}
          id={id}
          type="text"
          {...(props.secret === true ? { "data-secret": "true" } : {})}
          value={props.value}
          placeholder={props.placeholder}
          disabled={props.disabled}
          autoComplete="off"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          aria-invalid={props.problem === undefined ? undefined : true}
          onChange={(e) => {
            props.onChange(e.target.value);
          }}
        />
        {props.extra}
      </div>
      <p className="key-field-hint ui-help">{props.hint}</p>
      {field?.hasSignIn === true && props.value === field.value && (
        <p className="key-field-hint ui-help">
          This address includes a user name and password, shown as ***@ and kept hidden. To change
          them, type the whole address again with the new ones; to remove them, type it without
          ***@.
        </p>
      )}
      {field?.environmentValue != null && (
        <p className="key-field-hint ui-help" data-level="short">
          This computer&rsquo;s own settings name {field.environmentValue} for this, and Vibe Work
          uses that instead of what is saved here.
        </p>
      )}
      {field?.problem != null && (
        <p className="key-field-hint ui-help" data-level="short">
          The saved value cannot be used: {field.problem}
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
      <p className="key-field-hint ui-help">
        For company networks. If Vibe Work cannot reach Mistral because your network sends internet
        traffic through a proxy, or checks it with its own certificate, set both here. Your IT
        department can tell you what to enter.
      </p>
      {!n.running && (
        <p className="key-line" role="status">
          Vibe is not running right now. You can look at the settings and test them, but saving
          needs Vibe.
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
              small
              disabled={n.busy !== null}
              onClick={n.read}
            >
              Try again
            </Button>
          </div>
        </div>
      )}
      {view === null && n.error === null && <p className="key-line">Reading the settings...</p>}
      {view !== null && (
        <>
          {view.restartNeeded && n.note === null && (
            <p className="key-line" role="status">
              Saved settings apply the next time Vibe Work starts. Close it and open it again.
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
              hint="Where Vibe Work sends https requests, which is nearly all of them. For example http://proxy.example.com:8080. If the proxy needs a sign-in, write it before the host: http://user:password@proxy.example.com:8080."
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
              hint="For plain http requests. Most companies name the same address as above."
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
              hint="Used for requests the two fields above do not cover."
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
              hint={
                <>
                  Separate them with commas, for example .intranet.example.com, 10.0.0.0/8.
                  Addresses of this computer ({view.alwaysDirect.join(", ")}) always skip the proxy:
                  Vibe reaches Vibe Work through them.
                </>
              }
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
              hint="If your company checks web traffic, its proxy signs connections with its own certificate and Vibe Work must trust it. Choose that certificate file (a .pem file, from your IT department). It is added to the certificates already trusted."
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
                    small
                    disabled={locked}
                    onClick={n.chooseCertificate}
                  >
                    Choose file...
                  </Button>
                  {n.form.certificateFile !== "" && (
                    <Button
                      data-testid="settings.network-settings-clear-certificate"
                      small
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
                    ? "This file holds 1 certificate."
                    : `This file holds ${String(view.certificateCount)} certificates.`}
                </p>
              )}
            {(view.certificateFolder.value !== "" || n.form.certificateFolder !== "") && (
              <Field
                testId="settings.network-settings-set-certificate-folder"
                label="Company certificate folder"
                hint="A folder of certificate files, set outside Vibe Work (for example in the Vibe command line). Clear it to stop using it."
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
              Testing the connection to Mistral...
            </p>
          )}
          {n.outcome !== null && (
            <div className={n.outcome.reached ? "key-success" : "key-problem"} role="status">
              <p>{n.outcome.summary}</p>
              {n.outcome.nextStep !== null && <p className="ui-help">{n.outcome.nextStep}</p>}
              {n.outcome.overriddenByEnvironment.length > 0 && (
                <p className="ui-help">
                  This computer&rsquo;s own settings were used for:{" "}
                  {n.outcome.overriddenByEnvironment.join(", ")}.
                </p>
              )}
            </div>
          )}
          <div className="set-actions">
            <Button
              data-testid="settings.network-settings-test-connection"
              small
              disabled={locked}
              onClick={n.testConnection}
            >
              Test connection
            </Button>
            <Button
              data-testid="settings.network-settings-save"
              small
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
