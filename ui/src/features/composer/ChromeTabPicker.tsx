/**
 * Connected Chrome tabs, using the composer's menu styles: a section heading with the icon close button, then one
 * row per tab (the title as plain menu text, the profile and address muted below). Rows are buttons, so
 * `.chrome-tabs .it` resets the browser's button look. Depends on the composer store and chromeTabs; called by
 * Composer.
 */
import { Button } from "../../components/Controls";
import { Icon } from "../../components/Icon";
import type { ReactElement } from "react";
import { useApp } from "../shell/AppContext";
import { useComposerState } from "./hooks";
import { pickChromeTab } from "./chromeTabs";

export function ChromeTabPicker({ sessionId }: { sessionId: string }): ReactElement | null {
  const app = useApp();
  const picker = useComposerState((s) => s.chromePicker);
  if (picker?.sessionId !== sessionId) return null;
  return (
    <div
      className="slash chrome-tabs"
      role="region"
      aria-label="Chrome tabs"
      data-testid="composer.chrome-tabs"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          app.composer.setChromePicker(null);
        }
      }}
    >
      <div className="chrome-tabs-head">
        <div className="secl">Chrome tabs</div>
        <Button
          type="button"
          appearance="icon"
          aria-label="Close Chrome tabs"
          title="Close Chrome tabs"
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
      ) : picker.rows.length === 0 ? (
        <div className="slash-empty" role="status">
          No Chrome tabs
        </div>
      ) : null}
      <div className="slash-list">
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
