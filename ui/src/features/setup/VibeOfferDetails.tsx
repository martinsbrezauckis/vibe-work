/**
 * What a Vibe install offer will do: the folder, the official sources with their checksums,
 * the pinned packages, the sizes and the exact commands. Shown before the user agrees, at setup and in the Vibe
 * update dialog, so both consents show the same facts.
 * Depends on: AppContext (link opener, toasts), protocol types. Called by: VibeInstallStep, VibeUpdateDialog.
 * Invariants: render only; links open in the system browser through the host's link opener.
 */
import { useState } from "react";
import type { InstallOffer } from "../../protocol/core";
import { useApp } from "../shell/AppContext";

const mb = (bytes: number) => `${String(Math.round(bytes / (1024 * 1024)))} MB`;

/** A link that opens in the system browser. */
export function Link({ url, label }: { url: string; label: string }) {
  const { core } = useApp();
  const [fallback, setFallback] = useState(false);
  return (
    <>
      <button
        data-testid="setup.vibe-offer-details-open-link"
        type="button"
        className="key-link"
        onClick={() => {
          core?.client.openLink(url).catch(() => {
            setFallback(true);
          });
        }}
      >
        {label}
      </button>
      {fallback && <span role="status"> Open {url} in your browser.</span>}
    </>
  );
}

/** The offer's facts. */
export function OfferDetails({ offer }: { offer: InstallOffer }) {
  return (
    <>
      <p className="key-field-hint">Folder: {offer.folder}</p>
      <ul className="vibe-sources">
        {offer.sources.map((source) => (
          <li key={source.name}>
            <Link url={source.url} label={source.name} />
            {source.sha256 !== null && <code className="vibe-sha">SHA-256 {source.sha256}</code>}
          </li>
        ))}
      </ul>
      <p>
        All {offer.packages} packages are pinned with their checksums (requirements SHA-256{" "}
        <code>{offer.requirementsSha256.slice(0, 16)}…</code>). About {mb(offer.downloadBytes)} to
        download, {mb(offer.diskBytes)} on disk.{" "}
        <Link url={offer.docsUrl} label="Vibe's documentation" />
      </p>
      <details>
        <summary data-testid="setup.vibe-offer-details-summary">Commands that will run</summary>
        <ol className="vibe-commands">
          {offer.steps.map((step) => (
            <li key={step}>
              <code>{step}</code>
            </li>
          ))}
        </ol>
      </details>
    </>
  );
}
