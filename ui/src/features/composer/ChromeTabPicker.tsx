/**
 * Connected browser tabs, using the composer's menu styles: a section heading with the icon close button, then one
 * row per tab (the title as plain menu text, the profile's shown name and address muted below). Rows are the shared
 * menu-item buttons. Once the chat uses a browser profile, tabs of other profiles
 * stay listed but disabled while its external connection is live. A released connection allows another selection.
 * Depends on the composer store and chromeTabs; called by
 * Composer. The menu fits between the chat header and footer, resizing its scrollable list as the window changes.
 * Keyboard: opened from its "Add a browser tab…" button, focus moves into the panel; Escape or the close button
 * closes it and puts focus back on that button.
 */
import { pickBuiltinTab } from "./builtinTabs";
import { Button } from "../../components/Controls";
import { Tabs, Tab } from "../../components/Tabs";
import { browserTabsPanelId, usableBrowsers, openBrowserTabs } from "./browserTabs";
import { useCoreState } from "../shell/hooks";
import { Icon } from "../../components/Icon";
import { type ReactElement, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useApp } from "../shell/AppContext";
import { useComposerState } from "./hooks";
import { chatProfile, pickChromeTab, sameProfile } from "./chromeTabs";
import { NEW_CHAT } from "../../state/newChatKey";
import { useShellState } from "../shell/hooks";
import { TEXT, wording } from "../../vocabulary";

/** The composer's "Add a browser tab…" button that opens this panel. */
function triggerOf(panel: HTMLElement | null): HTMLElement | null {
  return (
    panel
      ?.closest(".composer")
      ?.querySelector<HTMLElement>('[data-testid="composer.browser-tabs-open"]') ?? null
  );
}

/** The browser a profile identity belongs to; a bare UUID is a Chrome identity from before namespaces. */
function profileBrowser(id: string): "Chrome" | "Edge" | "Safari" | "Firefox" {
  if (id.startsWith("edge:")) return "Edge";
  if (id.startsWith("safari:")) return "Safari";
  if (id.startsWith("firefox:")) return "Firefox";
  return "Chrome";
}

/**
 * The open tab list of the picked browser for `sessionId`, with a browser switch when several are usable. Renders
 * nothing unless this chat's picker is open; picking a row adds that tab to the chat.
 */
export function ChromeTabPicker({ sessionId }: { sessionId: string }): ReactElement | null {
  const app = useApp();
  const view = useCoreState((s) => s.chromeControl);
  const browsers = usableBrowsers(view);
  const picker = useComposerState((s) => s.chromePicker);
  const newChatOwner = useShellState((s) => s.newChatOwner);
  const owner = sessionId === NEW_CHAT ? newChatOwner : sessionId;
  const bound = chatProfile(view, owner);
  const selection = view?.chatProfiles?.find((row) => row.sessionId === owner);
  const pendingRelease = selection?.state === "stopping" || selection?.state === "unconfirmed";
  const element = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<{ up: boolean; maxHeight?: number }>({ up: true });
  const open = picker?.sessionId === sessionId;
  const close = () => {
    const back = triggerOf(element.current);
    app.composer.setChromePicker(null);
    back?.focus();
  };
  useEffect(() => {
    // Only a panel opened from its button takes focus; an open started elsewhere leaves focus where it is.
    const panel = element.current;
    if (open && panel !== null && document.activeElement === triggerOf(panel)) panel.focus();
  }, [open]);
  useEffect(() => {
    if (picker?.sessionId === sessionId && !browsers.includes(picker.browser ?? "Chrome"))
      app.composer.setChromePicker(null);
  }, [app, picker, sessionId, browsers]);
  useLayoutEffect(() => {
    if (!open) return;
    const composer = element.current?.closest(".composer");
    const content = composer?.closest(".content");
    if (!composer || !content) return;
    const measure = () => {
      const anchor = composer.getBoundingClientRect();
      const frame = content.getBoundingClientRect();
      const header = content.querySelector(".shead")?.getBoundingClientRect();
      const above = anchor.top - Math.max(8, header?.bottom ?? frame.top) - 8;
      const below = Math.min(window.innerHeight - 8, frame.bottom) - anchor.bottom - 8;
      // Keep a useful list above the composer when it fits; otherwise use the larger space below.
      const up = above >= 200 || above >= below;
      setPlacement({
        up,
        maxHeight: Math.max(0, up ? above : below),
      });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(composer);
    observer.observe(content);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [open]);
  if (picker?.sessionId !== sessionId) return null;
  const browser = picker.browser ?? "Chrome";
  // A live external connection keeps its exact profile until the core confirms it has ended.
  const usedBy = bound === null ? null : profileBrowser(bound);
  const boundName = (selection?.name ?? "").replace(`${usedBy ?? ""} · `, "");
  const currentProfile = `${usedBy ?? browser}${boundName ? ` (${boundName})` : ""}`;
  const otherBrowser =
    usedBy !== null && usedBy !== browser
      ? wording(TEXT.browserProfileInUse, { profile: currentProfile })
      : null;
  return (
    <div
      ref={element}
      // Geometry exception: only the measured space between composer, header and viewport is inline.
      style={{ maxHeight: placement.maxHeight }}
      data-side={placement.up ? "above" : "below"}
      className="slash chrome-tabs"
      role="region"
      id={browserTabsPanelId(sessionId)}
      tabIndex={-1}
      aria-label={`${browser} tabs`}
      data-testid="composer.chrome-tabs"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          close();
        }
      }}
    >
      {browsers.length > 1 ? (
        <Tabs variant="segmented" aria-label="Browser" data-testid="composer.browser-tabs-browsers">
          {browsers.map((name) => (
            <Tab
              key={name}
              data-testid="composer.browser-tabs-switch"
              data-item-id={name}
              aria-selected={browser === name}
              onClick={() => void openBrowserTabs(app, sessionId, "window", name)}
            >
              {name}
            </Tab>
          ))}
        </Tabs>
      ) : null}
      <div className="chrome-tabs-head">
        <div className="secl">{browser} tabs</div>
        <Button
          type="button"
          appearance="icon"
          aria-label={`Close ${browser} tabs`}
          title={`Close ${browser} tabs`}
          data-testid="composer.chrome-tabs-close"
          onClick={close}
        >
          <Icon name="x" size={12} />
        </Button>
      </div>
      {picker.error !== null ? (
        <div className="key-problem" role="alert">
          {picker.error}
        </div>
      ) : picker.loading ? (
        <div className="slash-empty" role="status">
          Loading…
        </div>
      ) : picker.rows.length === 0 && (picker.builtinRows?.length ?? 0) === 0 ? (
        <div className="slash-empty" role="status">
          No tabs are open in {browser}.
        </div>
      ) : null}
      <div className="slash-list">
        {picker.builtinRows?.map((tab) => (
          <Button
            type="button"
            appearance="menu"
            key={tab.pageKey}
            data-testid="composer.builtin-tab-pick"
            data-item-id={tab.pageKey}
            disabled={picker.loading}
            onClick={() => {
              void pickBuiltinTab(app, sessionId, tab.pageKey);
            }}
          >
            <span className="chrome-tab-title">{tab.preview?.title || tab.preview?.url}</span>
            <span className="ui-help">{tab.preview?.url}</span>
          </Button>
        ))}
        {picker.rows.map((tab) => {
          const other =
            selection?.state !== "unopened" && bound !== null && !sameProfile(tab.profileId, bound);
          const blocked = other || pendingRelease;
          const detail = `${tab.profileName} · ${blocked ? (otherBrowser ?? TEXT.browserProfileOther) : tab.url}`;
          const reason = pendingRelease
            ? TEXT.browserProfileReleasePending
            : other
              ? wording(TEXT.browserProfileReleaseFirst, {
                  profile: currentProfile,
                  next: tab.profileName,
                })
              : null;
          return (
            <Button
              type="button"
              appearance="menu"
              data-testid="composer.chrome-tab-pick"
              data-item-id={`${tab.profileId}:${String(tab.tabId)}`}
              key={`${tab.profileId}:${String(tab.tabId)}`}
              // The name carries the second line (profile and address, or why the tab cannot be added), so
              // same-titled tabs of other profiles or sites sound different. It is in the name, not a description,
              // because WebKitGTK does not expose aria-description to screen readers.
              aria-label={`${tab.title || tab.url}, ${reason === null ? detail : `${tab.profileName} · ${reason}`}`}
              data-profile={blocked ? "other" : undefined}
              title={reason ?? undefined}
              disabled={picker.loading || blocked}
              onClick={() =>
                void pickChromeTab(
                  app,
                  sessionId,
                  { profileId: tab.profileId, tabId: tab.tabId },
                  "window",
                )
              }
            >
              <span className="chrome-tab-title">{tab.title || tab.url}</span>
              <span className="ui-help">{detail}</span>
            </Button>
          );
        })}
      </div>
    </div>
  );
}
