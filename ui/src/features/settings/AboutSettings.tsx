/**
 * Settings › About: name, version (from the core's `status`), the author with a contact address, the source link, the
 * project page on theshellx.com (Vibe Work is one of the ShellX family; the name stays Vibe Work), the
 * independence line and licence, and where data is kept (Settings rows, this computer's default folder only) with
 * Mistral's policy link.
 * Depends on: AppContext, hooks, components/BrandMark, components/Icon. Called by: SettingsPage; the constants are read
 * by settingsRoutes (the Debug API's About state).
 * Invariants: the contact opens the mail program through the host's `mailto:`-only opener (`openMailLink`); the
 * source link and the project page open in the system browser through the core's `openLink` (the same path as every
 * external link), never inside a webview; nothing here fetches either page.
 */
import { LinkButton } from "../../components/Controls";
import { type ReactElement, useState } from "react";

import { BrandMark } from "../../components/BrandMark";
import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { detectMac } from "../../state/shortcuts";

/** The author line. */
export const ABOUT_AUTHOR = "Made by Martins Brezauckis";
/** The contact address, and the only link the contact line may open (`mailto:` to this one address). */
const CONTACT_EMAIL = "martins.brezauckis@gmail.com";
export const CONTACT_URL = `mailto:${CONTACT_EMAIL}`;
/** Where the source code lives. */
export const SOURCE_URL = "https://github.com/martinsbrezauckis/vibe-work";
/** The project page (downloads, screenshots) on the ShellX family site. */
export const HOMEPAGE_URL = "https://theshellx.com/vibework";
/** Mistral's own policy; opened by the user, never fetched by this screen. */
export const MISTRAL_DATA_POLICY_URL = "https://legal.mistral.ai/terms/privacy-policy/";

/**
 * The default data folder on this computer's system. Only the default is known here: a custom folder or
 * `XDG_DATA_HOME` changes it, which the text says ("by default").
 */
function defaultDataFolder(): string {
  if (detectMac()) return "~/Library/Application Support/app.vibework.desktop";
  const nav = typeof navigator === "undefined" ? undefined : navigator;
  const platform =
    (nav as (Navigator & { userAgentData?: { platform?: string } }) | undefined)?.userAgentData
      ?.platform ??
    nav?.platform ??
    "";
  return /win/i.test(platform)
    ? "%APPDATA%\\app.vibework.desktop"
    : "~/.local/share/app.vibework.desktop";
}

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
            <span className="d">
              Independent project, not affiliated with Mistral AI. Open source under the MIT
              License.
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
          <LinkButton
            type="button"

            data-testid="settings.about-contact"
            data-href={CONTACT_URL}
            title={CONTACT_URL}
            onClick={() => {
              core?.client.openMailLink(CONTACT_URL).catch(() => {
                setContactFallback(true);
              });
            }}
          >
            {CONTACT_EMAIL}
          </LinkButton>
          {contactFallback && <span role="status"> Write to {CONTACT_EMAIL}.</span>}
        </span>
      </p>
      <p className="key-line">
        <ExternalLink
          testId="settings.about-source"
          url={SOURCE_URL}
          label="Source code on GitHub"
        />
        <span className="about-sep" aria-hidden="true">
          ·
        </span>
        <ExternalLink
          testId="settings.about-homepage"
          url={HOMEPAGE_URL}
          label="theshellx.com/vibework"
          description="Vibe Work's page in the ShellX family of apps"
        />
      </p>
      <div className="about-data" data-setting="data-retention">
        <h4>Data retention</h4>
        <div className="check">
          <span>
            Session history
            <span className="d">
              Vibe shares <code>~/.vibe/logs/session</code> with the Vibe command line (or uses your{" "}
              <code>VIBE_HOME</code> folder). With &quot;Keep Vibe Work sessions separate from the
              CLI&quot; on, it uses <code>vibe-home/logs/session</code> inside Vibe Work&apos;s data
              folder. Each choice shows a different chat list; chats in the other list are hidden,
              not deleted, until you switch back.
            </span>
          </span>
        </div>
        <div className="check">
          <span>
            Vibe Work&apos;s data
            <span className="d">
              Settings, project and review records, browser data and session copies stay in Vibe
              Work&apos;s data folder (by default <code>{defaultDataFolder()}</code>). Keys stay in
              the encrypted Vibe Work Vault folder beside it; captures are saved under{" "}
              <code>capture/</code> in the session&apos;s folder.
            </span>
          </span>
        </div>
        <div className="check">
          <span>
            Sent to Mistral
            <span className="d">
              Model requests, and any voice, OCR, Libraries or image requests you make. Vibe Work
              sends no telemetry of its own; Vibe&apos;s usage telemetry follows Settings › Privacy
              and data.
            </span>
          </span>
          <ExternalLink
            testId="settings.about-data-policy"
            url={MISTRAL_DATA_POLICY_URL}
            label="Mistral data policy"
          />
        </div>
      </div>
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
      <LinkButton
        type="button"

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
      </LinkButton>
      {fallback && <span role="status"> Open {url} in your browser.</span>}
    </>
  );
}
