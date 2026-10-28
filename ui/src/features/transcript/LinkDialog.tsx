/**
 * The link dialog (a trusted surface): a link in model output is shown here in full before
 * anything happens: the canonical URL (`model/links.ts`: punycode host, no lookalike text, the link's
 * own words play no part) and the host it goes to. "Open in browser" hands it to the core
 * (`open_link`), which opens the system browser; the main window never navigates. A link that cannot
 * be opened (not http/https, a hidden user name, malformed) says why and offers Copy only.
 * Depends on: model/links, useModalFocus, CopyButton, the transcript store. Called by: Transcript.
 * Invariants: modal (inert background, Escape closes, focus returns); nothing opens without a click on
 * "Open in browser".
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useCallback, useRef } from "react";

import { CopyButton } from "../../components/CopyButton";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useApp } from "../shell/AppContext";
import { useTranscriptUi } from "./hooks";
import { checkLink } from "./model/links";

function OpenDialog({ url }: { url: string }): ReactElement {
  const { transcript } = useApp();
  const dialog = useRef<HTMLDivElement>(null);
  const close = useCallback(() => {
    transcript.askLink(null);
  }, [transcript]);
  useModalFocus(true, dialog, close);
  const check = checkLink(url);
  return (
    <>
      <div data-testid="transcript.link-dialog-scrim" className="scrim" onClick={close} />
      <div
        data-testid="transcript.link-dialog-modal"
        className="modal link-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="link-dialog-title"
        ref={dialog}
        tabIndex={-1}
      >
        <header>
          <h2 id="link-dialog-title">
            {check.ok ? "Open this link in your browser?" : "This link is not opened"}
          </h2>
        </header>
        <section>
          <p className="mono link-full" data-testid="link-full">
            {check.url}
          </p>
          {check.ok ? (
            <p>
              It goes to <b className="link-host">{check.host}</b>. Check the address, not the words
              of the link.
            </p>
          ) : (
            <p className="link-refused">{check.reason}</p>
          )}
        </section>
        <footer className="modal-actions">
          <CopyButton
            testId="transcript.link-dialog-copybutton"
            text={check.url}
            label="Copy link"
          />
          <Button
            data-testid="transcript.link-dialog-tb-btn"
            type="button"

            data-dialog-initial-focus="true"
            aria-label="Cancel opening link (Escape)"
            title="Cancel opening link (Escape)"
            onClick={close}
          >
            Cancel
          </Button>
          {check.ok ? (
            <Button
              data-testid="transcript.link-dialog-open-link"
              type="button"
              appearance="dialog"
              aria-label="Open link in browser"
              title="Open link in browser"
              onClick={() => {
                void transcript.openLink(check.url);
              }}
            >
              Open in browser
            </Button>
          ) : null}
        </footer>
      </div>
    </>
  );
}

/** Renders the dialog while a link waits for confirmation. */
export function LinkDialog(): ReactElement | null {
  const link = useTranscriptUi((s) => s.link);
  return link === null ? null : <OpenDialog url={link} />;
}
