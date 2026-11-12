/**
 * Connected browser tabs, using the composer's menu styles: a section heading with the icon close button, then one
 * row per tab (the title as plain menu text, the profile and address muted below). Rows are buttons, so
 * `.chrome-tabs .it` resets the browser's button look. Depends on the composer store and chromeTabs; called by
 * Composer. The menu fits between the chat header and footer, resizing its scrollable list as the window changes.
 */
import { pickBuiltinTab } from "./builtinTabs";
import { Button } from "../../components/Controls";
import { Tabs, Tab } from "../../components/Tabs";
import { usableBrowsers, openBrowserTabs } from "./browserTabs";
import { useCoreState } from "../shell/hooks";
import { Icon } from "../../components/Icon";
import { type ReactElement, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useApp } from "../shell/AppContext";
import { useComposerState } from "./hooks";
import { pickChromeTab } from "./chromeTabs";

export function ChromeTabPicker({ sessionId }: { sessionId: string }): ReactElement | null {
  const app = useApp();
  const view = useCoreState((s) => s.chromeControl);
  const browsers = usableBrowsers(view);
  const picker = useComposerState((s) => s.chromePicker);
  const element = useRef<HTMLDivElement>(null);
  const [placement, setPlacement] = useState<{ up: boolean; maxHeight?: number }>({ up: true });
  const open = picker?.sessionId === sessionId;
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
  return (
    <div
      ref={element}
      // Geometry exception: only the measured space between composer, header and viewport is inline.
      style={{ maxHeight: placement.maxHeight }}
      data-side={placement.up ? "above" : "below"}
      className="slash chrome-tabs"
      role="region"
      aria-label={`${browser} tabs`}
      data-testid="composer.chrome-tabs"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          app.composer.setChromePicker(null);
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
          onClick={() => {
            app.composer.setChromePicker(null);
          }}
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
            appearance="menu-option"
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
        {picker.rows.map((tab) => (
          <Button
            type="button"
            appearance="menu-option"
            data-testid="composer.chrome-tab-pick"
            data-item-id={`${tab.profileId}:${String(tab.tabId)}`}
            key={`${tab.profileId}:${String(tab.tabId)}`}
            aria-label={tab.title || tab.url}
            disabled={picker.loading}
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
            <span className="ui-help">
              {tab.profileName} · {tab.url}
            </span>
          </Button>
        ))}
      </div>
    </div>
  );
}
