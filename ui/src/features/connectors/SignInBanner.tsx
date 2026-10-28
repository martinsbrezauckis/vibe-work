/**
 * The sign-in in progress and the sign-ins Vibe asked for during a turn, shown at the top
 * of the Connectors page: what is happening in plain words, "Open the sign-in page" (the link dialog
 * again), "Check now" (Mistral connectors) and "Stop waiting".
 * Depends on: hooks. Called by: ConnectorsPage.
 */
import { Button } from "../../components/Controls";
import type { ReactElement } from "react";

import { useKeyReadiness } from "../shell/KeyNotice";
import { useConnectorActions, useConnectors } from "./hooks";

const WAITING_CONNECTOR =
  "Finish the sign-in in your browser. Mistral keeps the account link; this page checks every few seconds.";
const WAITING_SERVER =
  "Finish the sign-in in your browser. The browser hands the sign-in to Vibe on this computer.";

/** The banner, or nothing. */
export function SignInBanner(): ReactElement | null {
  const keyBlocked = useKeyReadiness() !== null;
  const signIn = useConnectors((s) => s.signIn);
  const prompts = useConnectors((s) => s.prompts);
  const actions = useConnectorActions();
  if (signIn === null && prompts.length === 0) return null;
  return (
    <div className="csignin" role="status">
      {signIn === null ? null : (
        <div className={`csignin-row ${signIn.phase}`} data-phase={signIn.phase}>
          <b>{signIn.name}</b>
          <span>
            {signIn.phase === "starting"
              ? " Asking for the sign-in page…"
              : signIn.phase === "waiting"
                ? ` ${signIn.url === null ? "Waiting for the sign-in page…" : signIn.kind === "connector" ? WAITING_CONNECTOR : WAITING_SERVER}`
                : ` ${signIn.message ?? ""}`}
          </span>
          <span className="csignin-acts">
            {signIn.url !== null && signIn.phase === "waiting" ? (
              <Button
                data-testid="connectors.sign-in-banner-open-page"
                type="button"

                onClick={() => actions?.openPage()}
              >
                Open the sign-in page
              </Button>
            ) : null}
            {signIn.kind === "connector" && signIn.phase === "waiting" ? (
              <Button
                data-testid="connectors.sign-in-banner-check-now"
                type="button"

                disabled={keyBlocked}
                onClick={() => void actions?.checkNow()}
              >
                Check now
              </Button>
            ) : null}
            <Button
              data-testid="connectors.sign-in-banner-stop-waiting"
              type="button"

              onClick={() => void actions?.stopWaiting()}
            >
              {signIn.phase === "done" || signIn.phase === "failed" ? "Close" : "Stop waiting"}
            </Button>
          </span>
        </div>
      )}
      {prompts.map((p) => (
        <div data-item-id={p.id} key={p.id} className="csignin-row prompt" data-prompt={p.id}>
          <b>{p.name}</b>
          <span> needs you to sign in (a session asked).</span>
          <span className="csignin-acts">
            <Button
              data-testid="connectors.sign-in-banner-open-prompt"
              type="button"
              variant="primary"
              onClick={() => actions?.openPrompt(p.id)}
            >
              Sign in
            </Button>
          </span>
        </div>
      ))}
    </div>
  );
}
