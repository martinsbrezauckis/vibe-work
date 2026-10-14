/**
 * The Settings page body: the chosen category's settings in the content area, or, while the
 * search inside Settings has words, the matching settings grouped by category (each opens its category at the
 * setting). A layer over the view underneath: the session, its scroll, the Projects overview or the Tools page stay
 * mounted and come back unchanged when Settings closes (Back, Esc, or any rail item).
 * Depends on: AppContext, hooks, catalog, every settings section component, components/Highlight.
 * Called by: Content (while `settings.open`).
 * Invariants: a `target` (from a search hit) scrolls its `data-setting` row into view and lights it for 1.5 s, then
 * is cleared; a row that is not rendered yet (a section still reading its data) is retried for up to a second, then
 * the category heading lights instead. Esc goes Home (as the rail's Home does) unless focus is in a text field or a
 * menu; Ctrl F focuses
 * the search inside Settings.
 */
import { type ReactElement, useEffect, useRef } from "react";
import { openHit } from "./openHit";

import { Highlight } from "../../components/Highlight";
import { Icon } from "../../components/Icon";
import { NotificationSettings } from "../shell/NotificationSettings";
import { useApp } from "../shell/AppContext";
import { goHome } from "../shell/home";
import { useShellState } from "../shell/hooks";
import { KeySettings } from "../setup/KeySettings";
import { ModelSettings } from "../setup/ModelSettings";
import { UpdateSettings } from "../updates/UpdateSettings";
import { AboutSettings } from "./AboutSettings";
import { BrowserPoolSettings } from "./BrowserPoolSettings";
import { BrowserDelegationSettings } from "./BrowserDelegationSettings";
import { AgentDefaultsSettings } from "./AgentDefaultsSettings";
import { AccountSettings } from "./AccountSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { CliSharingSettings } from "./CliSharingSettings";
import { DeveloperSettings } from "./DeveloperSettings";
import { ErrorDetailSetting } from "./ErrorDetailSetting";
import { InstructionsSettings } from "./InstructionsSettings";
import { MistralLinks } from "./MistralLinkRows";
import { PrivacySettings } from "./PrivacySettings";
import { SignedInAccounts } from "./SignedInAccounts";
import { ProjectSecretsSettings } from "../secrets/ProjectSecretsSettings";
import { SafetySettings } from "../secrets/SafetySettings";
import { ImageSettings } from "./ImageSettings";
import { SETTINGS_SEARCH_ID } from "./SettingsNav";
import { VoiceSettings } from "./VoiceSettings";
import { useOfferedCategories } from "./availability";
import { searchSettings } from "./catalog";

/** The sections each category shows, in order. */
function sections(category: string): ReactElement {
  switch (category) {
    case "general":
      return (
        <>
          <CliSharingSettings />
          <ErrorDetailSetting />
        </>
      );
    case "appearance":
      return <AppearanceSettings />;
    case "account":
      return (
        <>
          <AccountSettings part="account" />
          <KeySettings />
          <MistralLinks category="account" />
        </>
      );
    case "usage":
      return (
        <>
          {/* This conversation's tokens and context; the per-minute rate limit joins here with. */}
          <AccountSettings part="usage" />
          <MistralLinks category="usage" />
        </>
      );
    case "secrets":
      return <ProjectSecretsSettings />;
    case "models":
      return (
        <>
          <ModelSettings />
          <AgentDefaultsSettings />
          <ImageSettings />
        </>
      );
    case "browser":
      return (
        <>
          <BrowserPoolSettings />
          <BrowserDelegationSettings />
        </>
      );
    case "safety":
      return <SafetySettings />;
    case "instructions":
      return (
        <>
          <InstructionsSettings />
          <MistralLinks category="instructions" />
        </>
      );
    case "voice":
      return <VoiceSettings />;
    case "notifications":
      return <NotificationSettings />;
    case "privacy":
      return (
        <>
          <SignedInAccounts />
          <PrivacySettings />
          <MistralLinks category="privacy" />
        </>
      );
    case "updates":
      return <UpdateSettings />;
    case "developer":
      return <DeveloperSettings />;
    default:
      return <AboutSettings />;
  }
}

/** How long a found setting stays lit. */
const FLASH_MS = 1500;

/** Lights up the `target` row once it is on the page (retried while sections load), then forgets the target. */
function useTargetFlash(root: React.RefObject<HTMLDivElement | null>) {
  const { shell } = useApp();
  const target = useShellState((s) => (s.settings.open ? s.settings.target : null));
  const category = useShellState((s) => s.settings.category);
  useEffect(() => {
    if (target === null) return;
    let tries = 0;
    let frame = 0;
    const attempt = () => {
      const page = root.current;
      const row = page?.querySelector<HTMLElement>(`[data-setting="${target}"]`) ?? null;
      if (row === null && tries < 60) {
        tries += 1;
        frame = requestAnimationFrame(attempt);
        return;
      }
      const lit = row ?? page?.querySelector<HTMLElement>(".set-head") ?? null;
      if (lit !== null) {
        if (typeof lit.scrollIntoView === "function") lit.scrollIntoView({ block: "center" });
        lit.classList.add("set-flash");
        lit.setAttribute("data-flash", "true");
        const done = lit;
        setTimeout(() => {
          done.classList.remove("set-flash");
          done.removeAttribute("data-flash");
        }, FLASH_MS);
      }
      shell.actions.clearSettingsTarget();
    };
    frame = requestAnimationFrame(attempt);
    // Only the search for the row stops here: the light, once on, runs its 1.5 s even though the target is
    // cleared at once (clearing it is what re-runs this effect).
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [target, category, root, shell]);
}

/** Esc does what Home does (outside text fields and menus); Ctrl F focuses the search inside Settings. */
function useSettingsKeys() {
  const app = useApp();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        document.getElementById(SETTINGS_SEARCH_ID)?.focus();
        return;
      }
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const el = e.target instanceof HTMLElement ? e.target : null;
      const typing =
        el !== null &&
        el.id !== SETTINGS_SEARCH_ID &&
        (el.isContentEditable ||
          el.closest("input, textarea, select, [role=menu], [role=dialog]") !== null);
      if (typing) return;
      e.preventDefault();
      goHome(app);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [app]);
}

/** The page body. */
export function SettingsPage(): ReactElement {
  const app = useApp();
  const view = useShellState((s) => s.settings);
  const root = useRef<HTMLDivElement>(null);
  useTargetFlash(root);
  useSettingsKeys();
  const offered = useOfferedCategories();
  // A category this window does not offer (Updates without a channel) shows the first one instead.
  const category = offered.find((c) => c.id === view.category) ??
    offered[0] ?? { id: "general", label: "General" };
  const searching = view.query.trim() !== "";
  const results = searchSettings(view.query).filter((r) =>
    offered.some((c) => c.id === r.category),
  );
  return (
    <div className="settings-page" role="region" aria-label="Settings" ref={root}>
      <div className="set-body">
        {searching ? (
          <>
            <header className="set-head">
              <h2>Results</h2>
              <p className="set-path">
                {results.length === 1
                  ? "1 setting matches"
                  : `${String(results.length)} settings match`}{" "}
                “{view.query.trim()}”
              </p>
            </header>
            {offered
              .filter((c) => results.some((r) => r.category === c.id))
              .map((c) => (
                <div key={c.id} className="set-group">
                  <h3>{c.label}</h3>
                  {results
                    .filter((r) => r.category === c.id)
                    .map((r) => (
                      <button
                        key={r.id}
                        type="button"
                        className="set-hit"
                        onClick={() => {
                          openHit(app, r);
                        }}
                      >
                        <span>
                          <b>
                            <Highlight text={r.label} query={view.query} />
                          </b>
                          <span className="d">
                            <Highlight text={r.description} query={view.query} />
                          </span>
                        </span>
                        <Icon name="chevron-right" />
                      </button>
                    ))}
                </div>
              ))}
            {results.length === 0 && (
              <p className="key-line">
                No setting matches. Click “In: Settings” to search sessions and actions too.
              </p>
            )}
          </>
        ) : (
          <>
            <header className="set-head">
              <h2>{category.label}</h2>
              <p className="set-path">Settings › {category.label}</p>
            </header>
            <div
              className={`set-cat-body cat-${category.id}`}
              data-category={category.id}
              key={category.id}
            >
              {sections(category.id)}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
