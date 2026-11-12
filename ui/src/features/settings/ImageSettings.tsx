/**
 * Settings › Models and agent, "Images": what image generation sends to Mistral
 * and how many image requests this run sent. Information only: image generation asks nothing of its own
 * (Vibe's own approval for the session's access mode is the one ask, as for every other tool).
 * Depends on: AppContext (core), protocol/core (`images`). Called by: SettingsPage.
 * Invariants: shows the data-flow fact and the core's count; prices live on the provider's pricing page.
 */
import { type ReactElement, useEffect, useState } from "react";

import { LinkButton } from "../../components/Controls";
import type { ImagesResult } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";

/** The section. */
export function ImageSettings(): ReactElement | null {
  const blocked = useKeyReadiness() !== null;
  const { core } = useApp();
  const [state, setState] = useState<ImagesResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [linkFallback, setLinkFallback] = useState(false);
  useEffect(() => {
    if (blocked) return;
    let live = true;
    void core
      ?.run({ type: "images", request: { op: "read" } })
      .then((result) => {
        if (live && result.type === "images") {
          setState(result.result);
          setError(null);
        }
      })
      .catch(() => {
        if (live) setError("Could not read the image details.");
      });
    return () => {
      live = false;
    };
  }, [blocked, core]);
  if (core === null) return null;
  return (
    <section aria-labelledby="images-title">
      <h3 id="images-title">Images</h3>
      {blocked && (
        <KeyNotice
          inSettings
          message="Image generation uses your Mistral key, which is kept in the vault."
        />
      )}
      <div className="check" data-setting="image-generation">
        <span>
          Image generation
          <span className="d">
            Prompts and source images go to Mistral.
            {state === null || state.made === 0
              ? ""
              : ` Image requests since the app started: ${String(state.made)}.`}
          </span>
          <span className="d">
            <LinkButton
              data-testid="settings.image-settings-prevent-default"
              onClick={() => {
                void core.client.openLink("https://mistral.ai/pricing/").catch(() => {
                  setLinkFallback(true);
                });
              }}
            >
              Prices: Mistral&apos;s pricing page
            </LinkButton>
            {linkFallback && (
              <span role="status"> Open https://mistral.ai/pricing/ in your browser.</span>
            )}
          </span>
          {blocked || error === null ? null : (
            <span className="key-problem" role="alert">
              {error}
            </span>
          )}
        </span>
      </div>
    </section>
  );
}
