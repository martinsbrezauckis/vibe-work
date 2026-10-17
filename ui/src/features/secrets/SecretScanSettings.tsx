/** Session-log scan settings below Vault entries. Reuses Settings rows and notices; Rust owns saved switches. */
import { useId } from "react";
import { Button } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import { scanStatus } from "../../state/secretScanText";
import { useApp } from "../shell/AppContext";

/** The dependent checkbox and status exist only while scanning is enabled. */
export function SecretScanSettings() {
  const scan = useApp().secretScan;
  return scan === null ? null : <ScanSettings scan={scan} />;
}

function ScanSettings({ scan }: { scan: NonNullable<ReturnType<typeof useApp>["secretScan"]> }) {
  const { view, pending, error } = useStore(scan.store, (state) => state);
  const id = useId();
  const enabled = view?.settings.enabled ?? false;
  const busy = pending.includes("settings");
  const status = view === null ? null : scanStatus(view);
  return (
    <section className="set-group ui-body" data-setting="secret-scan" aria-label="Session logs">
      <h3>Session logs</h3>
      <label className="check" htmlFor={`${id}-scan`}>
        <span>
          <span id={`${id}-scan-label`}>Scan session logs for my secrets</span>
          <span className="d" id={`${id}-scan-description`}>
            Looks for your Vault secrets and website passwords in saved chats, in the background
            when the app starts. Short or simple ones are skipped: a guess could be tested against
            them.
          </span>
        </span>
        <span className="key-check">
          <input
            data-testid="secrets.scan-enabled"
            id={`${id}-scan`}
            aria-labelledby={`${id}-scan-label`}
            aria-describedby={`${id}-scan-description`}
            type="checkbox"
            checked={enabled}
            disabled={view === null}
            aria-disabled={busy}
            onChange={(event) => {
              void scan.run({
                op: "write",
                settings: { enabled: event.target.checked, autoRedact: false },
              });
            }}
          />
        </span>
      </label>
      {enabled && (
        <div className="key-remember" style={{ paddingInlineStart: "var(--space-card-inline)" }}>
          <label className="check" htmlFor={`${id}-auto`}>
            <span>
              <span id={`${id}-auto-label`}>Redact found secrets automatically</span>
              <span className="d" id={`${id}-auto-description`}>
                Replaces each one with [redacted]. Website passwords are never redacted
                automatically: you decide each one. Open chats are done when they close.
              </span>
            </span>
            <span className="key-check">
              <input
                data-testid="secrets.scan-auto-redact"
                id={`${id}-auto`}
                aria-labelledby={`${id}-auto-label`}
                aria-describedby={`${id}-auto-description`}
                type="checkbox"
                checked={view?.settings.autoRedact ?? false}
                aria-disabled={busy}
                onChange={(event) => {
                  void scan.run({
                    op: "write",
                    settings: { enabled: true, autoRedact: event.target.checked },
                  });
                }}
              />
            </span>
          </label>
          <p className="key-line" role="status">
            {status}
          </p>
          <div className="key-actions">
            <Button
              small
              data-testid="secrets.scan-check-now"
              aria-disabled={busy}
              onClick={() => {
                void scan.run({ op: "checkNow" });
              }}
            >
              Check now
            </Button>
          </div>
        </div>
      )}
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
