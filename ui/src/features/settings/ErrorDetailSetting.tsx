/**
 * Settings, "Errors": show the provider's own error text in the thread, or a plain line.
 * Depends on: state/errorDetail, state/createStore. Called by: settings/SettingsPage (the Settings dialog is gone).
 * Invariants: off by default; the preference is a per-computer display choice (it never changes what
 * Vibe or Mistral do), kept by `state/errorDetail`.
 */
import { errorDetail, setErrorDetail } from "../../state/errorDetail";
import { useStore } from "../../state/createStore";

/** The section. */
export function ErrorDetailSetting() {
  const mode = useStore(errorDetail, (m) => m);
  return (
    <section aria-labelledby="error-detail-title">
      <h3 id="error-detail-title">Errors</h3>
      <div className="check" data-setting="error-detail">
        <span>
          Show the provider&apos;s own error text
          <span className="d">
            When the Mistral API fails, the thread shows its status code and message. Off: a plain
            line that says what happened.
          </span>
        </span>
        <button
          data-testid="settings.error-detail-setting-set-error-detail"
          type="button"
          className="switch"
          role="switch"
          aria-label="Show the provider's own error text"
          aria-checked={mode === "details"}
          onClick={() => {
            setErrorDetail(mode === "details" ? "plain" : "details");
          }}
        />
      </div>
    </section>
  );
}
