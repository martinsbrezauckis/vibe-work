/**
 * Settings › About: name, version (from the core's `status`), the author with a contact address, the source link, the
 * project page on theshellx.com (Vibe Work is one of the ShellX family; the name stays Vibe Work), the
 * independence line and the licence. The contact and links are limited to the repository and project page.
 * Depends on: AppContext, hooks, components/BrandMark, components/Icon. Called by: SettingsPage; the constants are read
 * by settingsRoutes (the Debug API's About state).
 * Invariants: the contact opens the mail program through the host's `mailto:`-only opener (`openMailLink`); the
 * source link and the project page open in the system browser through the core's `openLink` (the same path as every
 * external link), never inside a webview; nothing here fetches either page.
 */
import { type ReactElement, useState } from "react";

import { BrandMark } from "../../components/BrandMark";
import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";

/** The author line. */
export const ABOUT_AUTHOR = "Made by Martins Brezauckis";
/** The contact address, and the only link the contact line may open (`mailto:` to this one address). */
export const CONTACT_EMAIL = "martins.brezauckis@gmail.com";
export const CONTACT_URL = `mailto:${CONTACT_EMAIL}`;
/** Where the source code lives. */
export const SOURCE_URL = "https://github.com/martinsbrezauckis/vibe-work";
/** The project page (downloads, screenshots) on the ShellX family site. */
export const HOMEPAGE_URL = "https://theshellx.com/vibework";

/** The About section. */
export function AboutSettings(): ReactElement {
  const { core } = useApp();
  const [contactFallback, setContactFallback] = useState(false);
  const appVersion = useCoreState((s) => s.appVersion);
  return (
    <section data-testid="about" aria-labelledby="about-title">
      <h3 id="about-title">About</h3>
      <div className="check about-row" data-setting="about">
        <span className="about-name">
          <BrandMark />
          <span>
            Vibe Work
            <span className="d" data-testid="about-version">
              {appVersion === null ? "Version unknown" : `Version ${appVersion}`}
            </span>
          </span>
        </span>
      </div>
      <p className="key-line" data-testid="about-author">
        {ABOUT_AUTHOR}
        <span className="about-sep" aria-hidden="true">
          ·
        </span>
        <span>
          Contact:{" "}
          <button
            type="button"
            className="ext-link"
            data-testid="about-contact"
            data-href={CONTACT_URL}
            title={CONTACT_URL}
            onClick={() => {
              core?.client.openMailLink(CONTACT_URL).catch(() => {
                setContactFallback(true);
              });
            }}
          >
            {CONTACT_EMAIL}
          </button>
          {contactFallback && <span role="status"> Write to {CONTACT_EMAIL}.</span>}
        </span>
      </p>
      <p className="key-line">
        <ExternalLink testId="about-source" url={SOURCE_URL} label="Source code on GitHub" />
        <span className="about-sep" aria-hidden="true">
          ·
        </span>
        <ExternalLink
          testId="about-homepage"
          url={HOMEPAGE_URL}
          label="theshellx.com/vibework"
          description="Vibe Work's page in the ShellX family of apps"
        />
      </p>
      <p className="key-field-hint">Independent project, not affiliated with Mistral AI.</p>
      <p className="key-field-hint">Open source under the MIT License.</p>
    </section>
  );
}

/**
 * One link out of the app: opens `url` in the system browser through the core's `openLink`; when that fails, local text
 * names the address so the user can open it by hand.
 */
function ExternalLink({
  testId,
  url,
  label,
  description,
}: {
  testId: string;
  url: string;
  label: string;
  description?: string;
}): ReactElement {
  const { core } = useApp();
  const [fallback, setFallback] = useState(false);
  return (
    <>
      <button
        type="button"
        className="ext-link"
        data-testid={testId}
        data-href={url}
        title={description === undefined ? url : `${description} (${url})`}
        onClick={() => {
          core?.client.openLink(url).catch(() => {
            setFallback(true);
          });
        }}
      >
        <Icon name="external-link" size={14} />
        {label}
      </button>
      {fallback && <span role="status"> Open {url} in your browser.</span>}
    </>
  );
}
