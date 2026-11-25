/**
 * The sites whose Local Network Access the person answered in the Vibe Work browser: a page from the web asks before it
 * reaches apps and devices on this computer or the local network, and the answer is kept per site. Each row shows
 * the site with its Allow / Block answer (changeable) and Forget (the site is asked again next time). Uses the typed
 * core (`localNetworkSites`, `setLocalNetworkSite`) and shows only confirmed values; an answer given in the native
 * question while the list is open (`browserChanged` naming no chat) is read at once. Sites refused because a request
 * did not say which open page sent it, or timed out, are listed as "Not answered" with Allow and Block (nothing is allowed by
 * default; the answer is the same decision as in the question). Rendered by BrowserSettings.
 *
 * Layout: the label and its description keep the row's first line; the empty state, the lists and the macOS
 * own-address line each take the row's full width below it (settings.css), so none of them squeezes the label.
 */
import { useEffect, useState } from "react";
import { Button } from "../../components/Controls";
import { ACT_ALLOW, ACT_BLOCK, ACT_FORGET } from "./actionLabels";
import { Selector } from "../../components/Selector";
import type { LocalNetworkSite, SiteAnswer } from "../../protocol/core";
import { toCallError } from "../../protocol/client/types";
import { detectMac } from "../../state/shortcuts";
import { useApp } from "../shell/AppContext";

const OPTIONS = [
  { value: "allow", label: "Allow" },
  { value: "block", label: "Block" },
] as const;

/** A failed change keeps the confirmed answer and reports the error to the Browser page. */
export function BrowserLocalNetworkSettings({
  onError,
}: {
  onError: (error: string | null) => void;
}) {
  const { core } = useApp();
  const [sites, setSites] = useState<LocalNetworkSite[] | null>(null);
  const [unanswered, setUnanswered] = useState<string[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    onError(error);
  }, [error, onError]);
  useEffect(() => {
    if (core === null) return;
    let live = true;
    const read = () => {
      void core
        .run({ type: "browser", request: { op: "localNetworkSites" } })
        .then((reply) => {
          if (live && reply.type === "browser" && reply.result.type === "localNetworkSites") {
            setSites(reply.result.sites);
            setUnanswered(reply.result.unanswered.map((u) => u.site));
          }
        })
        .catch((thrown: unknown) => {
          if (live)
            setError(
              `Could not read local network access: ${toCallError(thrown).error.message.replace(/\.$/, "")}. Try again.`,
            );
        });
    };
    read();
    const stop = core.listen((event) => {
      if (event.type === "browserChanged" && event.sessionId === null) read();
    });
    return () => {
      live = false;
      stop();
    };
  }, [core]);
  const change = (site: string, answer: SiteAnswer | null) => {
    if (core === null || busy !== null) return;
    setBusy(site);
    setError(null);
    void core
      .run({ type: "browser", request: { op: "setLocalNetworkSite", site, answer } })
      .then((reply) => {
        if (reply.type !== "browser" || reply.result.type !== "localNetworkSites")
          throw new Error("Local network access unavailable");
        setSites(reply.result.sites);
        setUnanswered(reply.result.unanswered.map((u) => u.site));
      })
      .catch((thrown: unknown) => {
        setError(
          `Could not change local network access for ${site}: ${toCallError(thrown).error.message.replace(/\.$/, "")}. Try again.`,
        );
      })
      .finally(() => {
        setBusy(null);
      });
  };
  if (core === null || sites === null) return null;
  return (
    <div className="check" data-setting="browser-local-network-sites">
      <span>
        Sites that can reach your local network
        <span className="d">
          A site from the web asks before it reaches apps and devices on this computer or your local
          network.
        </span>
      </span>
      {sites.length === 0 && unanswered.length === 0 && <p className="d">No site has asked yet.</p>}
      {unanswered.length > 0 && (
        <ul aria-label="Sites not answered">
          {unanswered.map((site) => (
            <li
              key={site}
              data-item-id={site}
              data-testid="settings.browser-local-network.unanswered"
            >
              <span>
                {site}
                <span className="d">Not answered</span>
              </span>
              <Button
                shape="outlined"
                disabled={busy !== null}
                aria-label={`${ACT_ALLOW} ${site}`}
                data-testid="settings.browser-local-network.allow"
                onClick={() => {
                  change(site, "allow");
                }}
              >
                {ACT_ALLOW}
              </Button>
              <Button
                shape="outlined"
                disabled={busy !== null}
                aria-label={`${ACT_BLOCK} ${site}`}
                data-testid="settings.browser-local-network.block"
                onClick={() => {
                  change(site, "block");
                }}
              >
                {ACT_BLOCK}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {sites.length > 0 && (
        <ul aria-label="Sites that can reach your local network">
          {sites.map(({ site, answer }) => (
            <li key={site} data-item-id={site}>
              <span>{site}</span>
              <Selector
                testId="settings.browser-local-network.answer"
                label={`Local network access for ${site}`}
                value={answer}
                options={OPTIONS}
                compact
                disabled={busy !== null}
                onChange={(value) => {
                  if (value !== answer) change(site, value);
                }}
              />
              <Button
                shape="ghost"
                disabled={busy !== null}
                aria-label={`${ACT_FORGET} ${site}`}
                data-testid="settings.browser-local-network.forget"
                onClick={() => {
                  change(site, null);
                }}
              >
                {ACT_FORGET}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {detectMac() && (
        <p className="d">
          On macOS, web pages cannot reach this Mac’s own network addresses. Localhost and this
          chat’s preview servers keep their existing access.
        </p>
      )}
    </div>
  );
}
