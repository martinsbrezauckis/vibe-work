/**
 * "Get a key": opens the Vibe key page (Code › Vibe CLI) through the host's link opener. A Vibe key uses the plan's included usage; the Studio API-key page makes pay-as-you-go keys. The host confirms the full address before opening it. The address is not printed
 * next to the link (it wrapped mid-word); when the opener is unavailable the address appears locally.
 * Depends on: AppContext, state/keyText. Called by: CreateStep, KeyStep, KeySettings.
 */
import { useState } from "react";
import { VIBE_KEY_URL } from "../settings/mistralLinks";
import { useApp } from "../shell/AppContext";

/** The link row. */
export function ConsoleLink() {
  const { core } = useApp();
  const [fallback, setFallback] = useState(false);
  return (
    <p className="key-field-hint">
      Need a key?{" "}
      <button
        data-testid="setup.console-link-open-link"
        type="button"
        className="key-link"
        data-href={VIBE_KEY_URL}
        onClick={() => {
          core?.client.openLink(VIBE_KEY_URL).catch(() => {
            setFallback(true);
          });
        }}
      >
        Get a Vibe key (Code › Vibe CLI)
      </button>
      {fallback && <span role="status"> Open {VIBE_KEY_URL} in your browser to create a key.</span>}
    </p>
  );
}
