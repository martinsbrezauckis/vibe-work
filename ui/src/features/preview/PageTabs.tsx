/**
 * Page tabs inside the Browser panel. The App preview is pinned; web tabs can be reordered and closed.
 * Adapted from ShellX.
 * Depends on: the core's page-tab snapshot and the Browser controller. Called by PreviewTab.
 */
import {
  useEffect,
  useRef,
  useState,
  type DragEvent,
  type KeyboardEvent,
  type ReactElement,
} from "react";

import type { PageTab } from "../../protocol/core";
import type { Browser, HandoffReview } from "./browser";
import { TabHandoffDialog } from "./TabHandoffDialog";

function tabLabel(tab: PageTab): string {
  if (tab.kind === "appPreview") return "App preview";
  return tab.preview?.title?.trim() || tab.preview?.url || "New tab";
}

function siteMark(tab: PageTab): string {
  if (tab.kind === "appPreview") return "⌂";
  try {
    return new URL(tab.preview?.url ?? "").hostname.charAt(0).toUpperCase() || "○";
  } catch {
    return "○";
  }
}

function safeFavicon(tab: PageTab): string | null {
  const value = tab.faviconUrl;
  return value?.startsWith("data:image/png;base64,") && value.length < 100_000 ? value : null;
}

export function PageTabs({
  browser,
  sessionId,
  tabs,
  busy,
}: {
  browser: Browser;
  sessionId: string;
  tabs: readonly PageTab[];
  busy: boolean;
}): ReactElement {
  const [dragged, setDragged] = useState<string | null>(null);
  const [review, setReview] = useState<HandoffReview | null>(null);
  const [handoffBusy, setHandoffBusy] = useState(false);
  const [handoffError, setHandoffError] = useState<string | null>(null);
  const reviewRequest = useRef(0);
  const active = tabs.find((tab) => tab.active);
  const identity = `${sessionId}:${active?.pageKey ?? ""}`;
  const [reviewIdentity, setReviewIdentity] = useState(identity);
  useEffect(() => {
    reviewRequest.current += 1;
  }, [identity]);
  if (reviewIdentity !== identity) {
    setReviewIdentity(identity);
    setReview(null);
    setHandoffError(null);
    setHandoffBusy(false);
  }
  const order = tabs.filter((tab) => tab.kind === "web").map((tab) => tab.pageKey);
  const activeIndex = Math.max(
    0,
    tabs.findIndex((tab) => tab.active),
  );
  const onKeys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (busy) return;
    const control = event.ctrlKey || event.metaKey;
    if (control && event.key.toLowerCase() === "t") {
      event.preventDefault();
      void browser.openTab(sessionId);
    } else if (control && event.key.toLowerCase() === "w") {
      event.preventDefault();
      const active = tabs[activeIndex];
      if (active?.kind !== "web") return;
      void browser.closeTab(sessionId, active.pageKey);
    } else if (
      (control && event.key === "Tab") ||
      event.key === "ArrowRight" ||
      event.key === "ArrowLeft"
    ) {
      if (tabs.length < 2) return;
      event.preventDefault();
      const delta = event.key === "ArrowLeft" || event.shiftKey ? -1 : 1;
      const next = tabs[(activeIndex + delta + tabs.length) % tabs.length];
      if (next) void browser.switchTab(sessionId, next.pageKey);
    }
  };
  const reorder = (source: string, target: string) => {
    if (source === target || !order.includes(source) || !order.includes(target)) return;
    const next = order.filter((id) => id !== source);
    next.splice(next.indexOf(target), 0, source);
    void browser.reorderTabs(sessionId, next);
  };
  return (
    <div className="browser-page-tabbar">
      <div
        data-testid="preview.page-tabs-browser-page-tabs"
        className="browser-page-tabs"
        role="tablist"
        aria-label="Browser pages"
        onKeyDown={onKeys}
      >
        {tabs.map((tab) => {
          const label = tabLabel(tab);
          const web = tab.kind === "web";
          return (
            <div
              data-testid="preview.page-tabs-on-drag-start-surface-1"
              data-item-id={tab.pageKey}
              key={tab.pageKey}
              className="browser-page-tab"
              data-kind={tab.kind}
              data-suspended={tab.preview?.state === "suspended"}
              data-dragging={dragged === tab.pageKey}
              draggable={web && !busy}
              onDragStart={(event: DragEvent<HTMLDivElement>) => {
                if (!web) return;
                setDragged(tab.pageKey);
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", tab.pageKey);
              }}
              onDragOver={(event) => {
                if (!web || !dragged) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
              }}
              onDrop={(event) => {
                event.preventDefault();
                const source = dragged || event.dataTransfer.getData("text/plain");
                reorder(source, tab.pageKey);
                setDragged(null);
              }}
              onDragEnd={() => {
                setDragged(null);
              }}
              onAuxClick={(event) => {
                if (web && event.button === 1 && !busy)
                  void browser.closeTab(sessionId, tab.pageKey);
              }}
            >
              <button
                data-testid="preview.page-tabs-switch-tab"
                type="button"
                role="tab"
                className="browser-page-tab-select"
                aria-selected={tab.active}
                aria-label={label}
                title={tab.preview?.url ?? label}
                disabled={busy}
                onClick={() => void browser.switchTab(sessionId, tab.pageKey)}
              >
                <span className="browser-page-tab-mark" aria-hidden="true">
                  {tab.preview?.loading ? (
                    "◌"
                  ) : safeFavicon(tab) ? (
                    <img src={safeFavicon(tab) ?? undefined} alt="" />
                  ) : (
                    siteMark(tab)
                  )}
                </span>
                <span className="browser-page-tab-title">{label}</span>
                {tab.agentCurrent && (
                  <span
                    className="browser-page-tab-agent"
                    title="Agent's current tab"
                    aria-label="Agent's current tab"
                  >
                    ●
                  </span>
                )}
                {tab.preview?.state === "suspended" && (
                  <span
                    className="browser-page-tab-paused"
                    title="Paused tab"
                    aria-label="Paused tab"
                  >
                    Ⅱ
                  </span>
                )}
              </button>
              {web && (
                <button
                  data-testid="preview.page-tabs-close-tab"
                  type="button"
                  className="browser-page-tab-close"
                  aria-label={`Close ${label}`}
                  title={`Close ${label}`}
                  disabled={busy}
                  onClick={() => void browser.closeTab(sessionId, tab.pageKey)}
                >
                  ×
                </button>
              )}
            </div>
          );
        })}
      </div>
      <button
        data-testid="preview.page-tabs-open-tab"
        type="button"
        className="browser-page-new"
        aria-label="New web tab"
        title="New web tab (Ctrl+T)"
        disabled={busy}
        onClick={() => void browser.openTab(sessionId)}
      >
        +
      </button>
      {active?.kind === "web" && active.ownerKind === "user" && (
        <button
          data-testid="preview.page-tabs-set-handoff-busy"
          type="button"
          className="browser-page-handoff"
          aria-label="Give tab to agent"
          title="Give this tab to the agent"
          disabled={busy || handoffBusy}
          onClick={() => {
            const request = ++reviewRequest.current;
            setHandoffBusy(true);
            setHandoffError(null);
            void browser
              .reviewHandoff(sessionId, active.pageKey)
              .then((next) => {
                if (reviewRequest.current === request) setReview(next);
              })
              .finally(() => {
                if (reviewRequest.current === request) setHandoffBusy(false);
              });
          }}
        >
          Give to agent
        </button>
      )}
      {active?.kind === "web" && active.ownerKind === "delegatedToAgent" && (
        <button
          data-testid="preview.page-tabs-take-back-tab"
          type="button"
          className="browser-page-handoff"
          aria-label="Take tab back from agent"
          title="Take this tab back from the agent"
          disabled={busy || handoffBusy}
          onClick={() => void browser.takeBackTab(sessionId, active.pageKey)}
        >
          Take back
        </button>
      )}
      {review && (
        <TabHandoffDialog
          review={review}
          busy={handoffBusy}
          onClose={() => {
            reviewRequest.current += 1;
            setReview(null);
            setHandoffError(null);
          }}
          error={handoffError}
          onConfirm={() => {
            const request = ++reviewRequest.current;
            setHandoffBusy(true);
            setHandoffError(null);
            void browser
              .handoffTab(sessionId, review)
              .then((success) => {
                if (reviewRequest.current !== request) return;
                if (success) setReview(null);
                else
                  setHandoffError(
                    browser.store.get().error[sessionId] ??
                      "The tab handoff failed. Review this page again.",
                  );
              })
              .finally(() => {
                if (reviewRequest.current === request) setHandoffBusy(false);
              });
          }}
        />
      )}
    </div>
  );
}
