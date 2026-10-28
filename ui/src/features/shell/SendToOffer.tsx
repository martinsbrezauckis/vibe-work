/**
 * The one-time question about the right-click menus, as one row on the setup screen and the start screen (an
 * update from a version without them shows it once on the start screen): Add turns the "Right-click menus" switch
 * on, Not now ends the question; both are remembered by the host. Depends on the shared switch state
 * (`state/sendTo`) and the `key-notice` row; called by SetupScreen and Content.
 * Invariant: nothing is written before Add; after either answer the row is gone for good (Settings keeps the
 * switch).
 */
import { Button } from "../../components/Controls";
import { sendToRequest, useSendTo } from "../../state/sendTo";
import { useApp } from "./AppContext";

/** The row while the host offers the question; a failed Add stays with the host's sentence. */
export function SendToOffer() {
  const { core } = useApp();
  const client = core?.client ?? null;
  const { view, pending, error } = useSendTo(client);
  if (client === null || view?.offer !== true) return null;
  return (
    <section
      className="key-line send-to-offer"
      aria-label="Right-click menus"
      data-testid="shell.send-to-offer"
    >
      <p>Add Send to Vibe Work to the right-click menus?</p>
      <Button
        data-testid="shell.send-to-offer-add"
        type="button"
        disabled={pending}
        onClick={() => void sendToRequest(client, { op: "set", on: true })}
      >
        Add
      </Button>
      <Button
        data-testid="shell.send-to-offer-not-now"
        type="button"
        disabled={pending}
        onClick={() => void sendToRequest(client, { op: "notNow" })}
      >
        Not now
      </Button>
      {error !== null && (
        <p className="key-problem" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
