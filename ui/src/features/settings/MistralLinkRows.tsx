/**
 * "On Mistral's website": a category's links to Mistral's own pages. Each row is the page's name (where on the page to
 * go is the Open button's tooltip, not a visible line) and opens the page in the system browser through the
 * core's `openLink` (validated again in the host; never inside the app).
 * Depends on: AppContext, components/Icon, mistralLinks. Called by: SettingsPage.
 * Invariants: the link target is on the button as `data-href` (tests read it without opening anything); nothing is
 * fetched; when the opener is unavailable (browser dev mode) a toast gives the address to copy.
 */
import { type ReactElement, useState } from "react";

import { Icon } from "../../components/Icon";
import { Badge, LinkButton } from "../../components/Controls";
import { useApp } from "../shell/AppContext";
import { linksOf } from "./mistralLinks";

/** The links of `category` (renders nothing when it has none). */
export function MistralLinks({ category }: { category: string }): ReactElement | null {
  const { core } = useApp();
  const [fallback, setFallback] = useState<string | null>(null);
  const links = linksOf(category);
  if (links.length === 0) return null;
  return (
    <section aria-labelledby={`links-${category}`} className="set-links">
      <h3 id={`links-${category}`}>On Mistral&apos;s website</h3>
      {links.map((link) => (
        <div data-item-id={link.id} key={link.id} className="check" data-setting={link.id}>
          <span>
            {link.label}
            {link.webOnly === true && (
              <>
                {" "}
                <Badge>Web only</Badge>
              </>
            )}
          </span>
          <LinkButton
            data-testid="settings.mistral-link-rows-open-link"
            type="button"

            data-href={link.url}
            title={`${link.note} Opens ${link.url} in your browser.`}
            aria-label={`Open ${link.label} on Mistral's website`}
            onClick={() => {
              core?.client.openLink(link.url).catch(() => {
                setFallback(link.id);
              });
            }}
          >
            Open
            <Icon name="external-link" size={14} />
          </LinkButton>
          {fallback === link.id && (
            <span className="d" role="status">
              Open {link.url} in your browser.
            </span>
          )}
        </div>
      ))}
    </section>
  );
}
