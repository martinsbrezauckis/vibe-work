/**
 * The Settings page body: the chosen category's settings in the content area, or, while the
 * search inside Settings has words, the matching settings grouped by category (each opens its category at the
 * setting). A layer over the view underneath: the session, its scroll, the Projects overview or the Tools page stay
 * mounted and come back unchanged when Settings closes (Back, Esc, or any rail item).
 * Depends on: AppContext, hooks, catalog, every settings section component, components/Highlight.
 * Called by: Content (while `settings.open`).
 * Invariants: a `target` (from a search hit) scrolls its `data-setting` row into view, moves focus to its first
 * control and lights it for 1.5 s, then is cleared; a row that is not rendered yet (a section still reading its data) is retried for up to a second, then
 * the category heading lights instead. While searching, the result line (the count, or no match) is said once by
 * the polite announcer after typing pauses. Esc goes Home (as the rail's Home does) unless focus is in a text field, a
 * menu or an open inline form; Ctrl F focuses
 * the search inside Settings.
 */
import { type ReactElement, Suspense, lazy, useEffect, useRef } from "react";
import { announce } from "../../state/announcer";
import { openHit } from "./openHit";
import { useSettingsDescriptions } from "./useSettingsDescriptions";

import { Highlight } from "../../components/Highlight";
import { formatKeys } from "../../state/shortcuts";
import { Icon } from "../../components/Icon";
import { NotificationSettings } from "../shell/NotificationSettings";
import { useApp } from "../shell/AppContext";
import { goHome } from "../shell/home";
import { useCoreState, useShellState } from "../shell/hooks";
import { MistralKeys } from "./MistralKeys";
import { KeySettings } from "../setup/KeySettings";
import { ModelSettings } from "../setup/ModelSettings";
import { UpdateSettings } from "../updates/UpdateSettings";
import { AboutSettings } from "./AboutSettings";
import { BrowserSettings } from "./BrowserSettings";
import { AgentDefaultsSettings } from "./AgentDefaultsSettings";
import { AccountSettings } from "./AccountSettings";
import { PlanUsageSettings } from "./PlanUsageSettings";
import { AccessibilitySettings } from "./AccessibilitySettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { CliSharingSettings } from "./CliSharingSettings";
import { KeepAwakeSettings } from "./KeepAwakeSettings";
import { KeyboardShortcutsSetting } from "./KeyboardShortcutsSetting";
import { SendToSettings } from "./SendToSettings";
import { CopiesSettings } from "./CopiesSettings";
import { ImportSettings } from "./ImportSettings";
import { GitSettings } from "./GitSettings";
import { DeveloperSettings } from "./DeveloperSettings";
import { ErrorDetailSetting } from "./ErrorDetailSetting";
import { InstructionsSettings } from "./InstructionsSettings";
import { MistralLinks } from "./MistralLinkRows";
import { NetworkSettings } from "./NetworkSettings";
import { PrivacySettings } from "./PrivacySettings";
import { SignedInAccounts } from "./SignedInAccounts";
import { SecretScanSettings } from "../secrets/SecretScanSettings";
import { ProjectSecretsSettings } from "../secrets/ProjectSecretsSettings";
import { AllowedLevels } from "./allowed/AllowedLevels";
import { AllowedPage } from "./allowed/AllowedPage";
import { SafetySettings } from "../secrets/SafetySettings";
import { ImageSettings } from "./ImageSettings";
import { SETTINGS_SEARCH_ID } from "./SettingsNav";
import { VoiceSettings } from "./VoiceSettings";
import { SaveBeforeSendSetting } from "./SaveBeforeSendSetting";
import { useOfferedCategories } from "./availability";
import { chromeAvailable } from "./chrome/chromeRows";
import { searchSettings } from "./catalog";

// Loaded when the category opens: its forms and the connector picker stay out of the window's first paint.
const KnowledgeSettings = lazy(() =>
  import("./KnowledgeSettings").then((m) => ({ default: m.KnowledgeSettings })),
);

/** The sections each category shows, in order. */
function sections(category: string): ReactElement {
  switch (category) {
    case "general":
      return (
        <>
          <CliSharingSettings />
          <ErrorDetailSetting />
          <SaveBeforeSendSetting />
          <KeyboardShortcutsSetting />
          <KeepAwakeSettings />
          <SendToSettings />
          {/* The two Git cards stay together: author details, then session copies. */}
          <GitSettings />
          <CopiesSettings />
          <ImportSettings />
        </>
      );
    case "accessibility":
      return <AccessibilitySettings />;
    case "appearance":
      return <AppearanceSettings />;
    case "account":
      return (
        <>
          <AccountSettings part="account" />
          <KeySettings />
          <MistralKeys />
          <MistralLinks category="account" />
        </>
      );
    case "usage":
      return (
        <>
          <PlanUsageSettings />
          {/* This conversation's tokens and context; the per-minute rate limit joins here with. */}
          <AccountSettings part="usage" />
          <MistralLinks category="usage" />
        </>
      );
    case "secrets":
      return (
        <>
          <ProjectSecretsSettings />
          <SecretScanSettings />
        </>
      );
    case "models":
      return (
        <>
          <ModelSettings />
          <AgentDefaultsSettings />
          <ImageSettings />
        </>
      );
    case "browser":
      return <BrowserSettings />;
    case "safety":
      return (
        <>
          <AllowedLevels />
          <SafetySettings />
        </>
      );
    case "allowed":
      // Everything allowed without asking: its own category directly under Safety.
      return <AllowedPage />;
    case "instructions":
      return (
        <>
          <InstructionsSettings />
          <MistralLinks category="instructions" />
        </>
      );
    case "knowledge":
      return (
        <Suspense fallback={null}>
          <KnowledgeSettings />
        </Suspense>
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
    case "network":
      return <NetworkSettings />;
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

/** Controls a found setting can take focus on. */
const FOCUSABLE =
  'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';

/** Moves focus to the first control of a found setting (or the setting itself), so the next key acts there. */
function focusInto(row: HTMLElement): void {
  const control = row.querySelector<HTMLElement>(FOCUSABLE);
  if (control !== null) {
    control.focus({ preventScroll: true });
    return;
  }
  if (!row.hasAttribute("tabindex")) row.tabIndex = -1;
  row.focus({ preventScroll: true });
}

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
        focusInto(lit);
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

/** Esc does what Home does (outside text fields, menus and open inline forms, `data-settings-form`, whose typed
 * text it would drop); Ctrl F focuses the search inside Settings. */
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
          el.closest(
            "input, textarea, select, [role=menu], [role=dialog], [data-settings-form]",
          ) !== null);
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

/**
 * How long typing must pause before the search result is said. VoiceOver re-reads the focused search field by
 * itself about a second after typing stops and cuts off whatever it is saying then, even an assertive message; a
 * line said after that re-read (and before VoiceOver's own hint) is heard in full. Other screen readers hear the
 * same single line.
 */
const SAY_RESULT_MS = 1500;

/**
 * Says the search result line once the typing pauses (the visible lines are no live regions). The wait starts
 * again on every keystroke (`query`), also while the line stays the same (no match), so it is never said during
 * the typing or during the re-read.
 */
function useSayResult(said: string | null, query: string) {
  useEffect(() => {
    if (said === null) return;
    const timer = setTimeout(() => {
      announce(said);
    }, SAY_RESULT_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [said, query]);
}

/** The page body. */
export function SettingsPage(): ReactElement {
  const app = useApp();
  const view = useShellState((s) => s.settings);
  const root = useRef<HTMLDivElement>(null);
  useSettingsDescriptions(root);
  useTargetFlash(root);
  useSettingsKeys();
  const offered = useOfferedCategories();
  // A category this window does not offer (Updates without a channel) shows the first one instead.
  const category = offered.find((c) => c.id === view.category) ??
    offered[0] ?? { id: "general", label: "General" };
  const searching = view.query.trim() !== "";
  const chrome = useCoreState((s) => s.chromeControl);
  const results = searchSettings(
    view.query,
    chromeAvailable(chrome),
    chrome?.edge !== undefined,
    chrome?.firefox !== undefined,
    chrome?.safari !== undefined,
  ).filter((r) => offered.some((c) => c.id === r.category));
  const count = `${results.length === 1 ? "1 setting matches" : `${String(results.length)} settings match`} “${view.query.trim()}”`;
  const none = `No setting matches. Press ${formatKeys("Mod+k")} to search everything.`;
  useSayResult(searching ? (results.length === 0 ? none : count) : null, view.query);
  return (
    <div className="settings-page ui-body" role="region" aria-label="Settings" ref={root}>
      <div className="set-body">
        {searching ? (
          <>
            <header className="set-head">
              <h2>Results</h2>
              <p className="set-path">{count}</p>
            </header>
            {offered
              .filter((c) => results.some((r) => r.category === c.id))
              .map((c) => (
                <div data-item-id={c.id} key={c.id} className="set-group">
                  <h3>{c.label}</h3>
                  {results
                    .filter((r) => r.category === c.id)
                    .map((r) => (
                      <button
                        data-testid="settings.settings-page-open-hit"
                        data-item-id={r.id}
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
            {results.length === 0 && <p className="key-line">{none}</p>}
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
