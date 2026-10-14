/**
 * The Browser tab's address bar: the user types a web address or a local server's address and presses
 * Enter. The same navigation policy as the agent's applies (the core refuses private, LAN, file and app-own targets
 * and other sessions' ports, with the reason shown as the tab's error). Shows the current URL while not edited.
 * Depends on: store.ts (`open`), browserModel.ts. Called by: PreviewTab inside Toolbar.
 */
import { type ReactElement, useState } from "react";

import { addressToUrl, isLocalPage } from "./browserModel";
import type { Browser } from "./browser";
import type { Previews } from "./store";

/** The editable address of `sessionId`'s browser. */
export function AddressBar({
  previews,
  browser,
  webTabId,
  sessionId,
  url,
  loading,
  busy,
  title,
}: {
  previews: Previews;
  browser?: Browser;
  webTabId?: string;
  sessionId: string;
  url: string;
  loading: boolean;
  busy: boolean;
  /** Page title, shown only as a tooltip; page text is never interpreted as markup. */
  title?: string | null;
}): ReactElement {
  // What the user is typing; `null` while not editing (the field shows the current URL).
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? url;
  return (
    <form
      className="preview-address-form"
      onSubmit={(e) => {
        e.preventDefault();
        const target = addressToUrl(text);
        if (target === "") return;
        setDraft(null);
        if (webTabId && browser) void browser.webAction(sessionId, webTabId, "navigate", target);
        else if (browser && !isLocalPage(target)) void browser.openTab(sessionId, target);
        else void previews.open(sessionId, { url: target });
      }}
    >
      <input
        className="preview-address"
        aria-label="Address"
        title={title ?? url}
        placeholder="Type a web address or a local server (localhost:5173)"
        spellCheck={false}
        data-loading={loading}
        disabled={busy}
        value={text}
        onBlur={() => {
          setDraft(null);
        }}
        onChange={(e) => {
          setDraft(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            setDraft(null);
            e.currentTarget.blur();
          }
        }}
      />
    </form>
  );
}
