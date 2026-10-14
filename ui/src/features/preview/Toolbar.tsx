/**
 * One browser toolbar: navigation, editable address, agent status, compact primary actions and overflow.
 * Narrow panels move secondary controls into the menu without wrapping or unmounting the vault chooser.
 * Depends on: shared IconButton/Selector, preview controller and browser overlay host. Called by: PreviewTab.
 * Invariant: external URLs still go through the link-review dialog; the native viewport stays mounted.
 */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactElement,
  type ReactNode,
} from "react";

import { IconButton } from "../../components/IconButton";
import { Selector } from "../../components/Selector";
import type { DeviceWidth, PreviewView } from "../../protocol/core";
import type { Previews } from "./store";
import type { Browser } from "./browser";
import { browserMenuKeys, useBrowserPopup } from "./useBrowserPopup";
import { closeBrowserOverlaysAndWait, registerBrowserOverlay } from "./browserOverlays";

const WIDTHS: ReadonlyArray<{ width: DeviceWidth; label: string }> = [
  { width: "phone", label: "390" },
  { width: "tablet", label: "768" },
  { width: "desktop", label: "1280" },
  { width: "fit", label: "Fit" },
];

/** The toolbar remains available even before the session has opened a page. */
export function Toolbar({
  previews,
  browser,
  webTabId,
  preview,
  sessionId,
  busy,
  error = null,
  openExternal,
  point,
  address,
  activity,
  save,
  capture,
  annotate,
  actions,
  servers,
}: {
  previews: Previews;
  browser?: Browser;
  webTabId?: string;
  preview: PreviewView | null;
  sessionId: string;
  busy: boolean;
  error?: string | null;
  openExternal: (url: string) => void;
  point: (() => void) | null;
  address: ReactNode;
  activity: ReactNode;
  save: (collapsed: boolean, close: () => void, visible: boolean) => ReactNode;
  capture: ReactNode;
  annotate: ((mode: "element" | "box") => void) | null;
  actions: ReactNode;
  servers: ReactNode;
}): ReactElement {
  const bar = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const [collapsed, setCollapsed] = useState(false);
  const [panel, setPanel] = useState<"menu" | "actions" | "servers" | null>(null);
  const [widthOpen, setWidthOpen] = useState(false);
  const widthOpenRef = useRef(false);
  const close = useCallback(() => {
    setPanel(null);
  }, []);
  const closeOutside = useCallback(() => {
    if (!widthOpenRef.current) setPanel(null);
  }, []);
  useEffect(() => {
    const element = bar.current;
    if (element === null) return;
    const measure = () => {
      const width = element.getBoundingClientRect().width;
      // Zero is an unmeasured/hidden panel; keep its current mode until it has real bounds.
      if (width > 0) setCollapsed(width < 600);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    window.addEventListener("resize", measure);
    measure();
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);
  const popup = useBrowserPopup(
    `browser-toolbar-${panel ?? "closed"}-${String(collapsed)}`,
    panel !== null,
    trigger,
    closeOutside,
  );
  useLayoutEffect(() => {
    if (!widthOpen) return;
    const element = document.querySelector<HTMLDivElement>(
      '.ui-selector-menu[aria-label="Device width"]',
    );
    if (element === null) return;
    return registerBrowserOverlay("browser-device-width-selector", {
      element,
      trigger: document.querySelector<HTMLElement>('[role="combobox"][aria-label="Device width"]'),
      close: () => {
        element.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
        );
      },
    });
  }, [widthOpen]);
  const url = preview?.url ?? "";
  const open = url !== "";
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if ((event.target as HTMLElement).closest('[aria-modal="true"]') !== null) return;
    if ((event.target as HTMLElement).closest('[role="combobox"]') !== null) return;
    if (event.key === "Escape" && !event.defaultPrevented) {
      event.preventDefault();
      event.stopPropagation();
      close();
      trigger.current?.focus();
    } else browserMenuKeys(event);
  };
  const item = (
    label: string,
    click: () => void,
    disabled = false,
    title = label,
    needsPage = false,
  ) => (
    <button
      type="button"
      className="browser-menu-item"
      role="menuitem"
      title={title}
      disabled={disabled}
      onClick={() => {
        close();
        if (needsPage)
          void closeBrowserOverlaysAndWait()
            .then(click)
            .catch(() => undefined);
        else click();
      }}
    >
      {label}
    </button>
  );
  return (
    <div className="browser-chrome-shell">
      <div
        ref={bar}
        className="preview-toolbar"
        role="toolbar"
        aria-label="Preview"
        data-collapsed={collapsed}
      >
        <span className="browser-navigation">
          <IconButton
            icon="arrow-left"
            label="Back"
            disabled={busy || !open}
            onClick={() =>
              void (webTabId && browser
                ? browser.webAction(sessionId, webTabId, "back")
                : previews.back(sessionId))
            }
          />
          <IconButton
            icon="arrow-right"
            label="Forward"
            disabled={busy || !open}
            onClick={() =>
              void (webTabId && browser
                ? browser.webAction(sessionId, webTabId, "forward")
                : previews.forward(sessionId))
            }
          />
          <IconButton
            icon="refresh-cw"
            label="Reload"
            disabled={busy || !open}
            onClick={() =>
              void (webTabId && browser
                ? browser.webAction(sessionId, webTabId, "reload")
                : previews.reload(sessionId))
            }
          />
        </span>
        {address}
        {activity}
        <div className="browser-controls">
          <div
            className="browser-controls-content"
            data-collapsed={collapsed}
            hidden={collapsed && panel !== "menu"}
            ref={collapsed && panel === "menu" ? popup : undefined}
            role={collapsed ? "menu" : undefined}
            aria-label={collapsed ? "Browser options" : undefined}
            onKeyDown={keys}
          >
            <div className="browser-secondary">
              {(!webTabId || browser) && (
                <div className="browser-width-control">
                  <Selector
                    label="Device width"
                    ariaDescription={`Current width: ${WIDTHS.find((w) => w.width === (preview?.width ?? "fit"))?.label ?? "Fit"}`}
                    value={preview?.width ?? "fit"}
                    options={WIDTHS.map((w) => ({
                      value: w.width,
                      label: w.label,
                      icon: "monitor",
                      description: w.width === "fit" ? "Fit the panel" : `${w.label} px wide`,
                    }))}
                    onChange={(width) => {
                      close();
                      void closeBrowserOverlaysAndWait()
                        .then(async () => {
                          if (webTabId && browser)
                            await browser.setWebWidth(sessionId, webTabId, width);
                          else await previews.setWidth(sessionId, width);
                        })
                        .catch(() => undefined);
                    }}
                    onOpen={() => {
                      widthOpenRef.current = true;
                      setWidthOpen(true);
                    }}
                    onClose={() => {
                      widthOpenRef.current = false;
                      setWidthOpen(false);
                    }}
                    disabled={busy || !open || (collapsed && panel !== "menu")}
                    title="Device width"
                    compact
                  />
                </div>
              )}
              {collapsed ? (
                item(
                  "Open in the system browser",
                  () => {
                    openExternal(url);
                  },
                  !open,
                )
              ) : (
                <IconButton
                  icon="external-link"
                  label="Open in the system browser"
                  disabled={!open}
                  onClick={() => {
                    openExternal(url);
                  }}
                />
              )}
              {point !== null &&
                (collapsed ? (
                  item("Point Vibe at this", point, busy || !open, "Point Vibe at this", true)
                ) : (
                  <IconButton
                    icon="crosshair"
                    label="Point Vibe at this"
                    disabled={busy || !open}
                    onClick={point}
                  />
                ))}
              {save(collapsed, close, !collapsed || panel === "menu")}
            </div>
            {panel === "menu" && (
              <div
                className="browser-overflow-items"
                ref={collapsed ? undefined : popup}
                role={collapsed ? undefined : "menu"}
                aria-label={collapsed ? undefined : "Browser options"}
                onKeyDown={keys}
              >
                {capture}
                {annotate !== null && (
                  <>
                    {item(
                      "Note on an element",
                      () => {
                        annotate("element");
                      },
                      busy,
                      "Click an element in the preview to add a note about it",
                      true,
                    )}
                    {item(
                      "Note on an area",
                      () => {
                        annotate("box");
                      },
                      busy,
                      "Drag a box in the preview to add a note about that area",
                      true,
                    )}
                  </>
                )}
                <button
                  type="button"
                  role="menuitem"
                  className="browser-menu-item"
                  title="Browser actions"
                  onClick={() => {
                    setPanel("actions");
                  }}
                >
                  Browser actions
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="browser-menu-item"
                  title="This session's servers"
                  onClick={() => {
                    setPanel("servers");
                  }}
                >
                  This session&apos;s servers
                </button>
                {webTabId && browser
                  ? item(
                      "Close this tab",
                      () => void browser.closeTab(sessionId, webTabId),
                      busy,
                      "Close this tab",
                      true,
                    )
                  : item(
                      "Close the preview",
                      () => void previews.close(sessionId),
                      busy || !open,
                      "Close the preview",
                      true,
                    )}
              </div>
            )}
          </div>
          <IconButton
            ref={trigger}
            icon="ellipsis"
            label="Browser options"
            aria-haspopup="menu"
            aria-expanded={panel !== null}
            onClick={() => {
              setPanel(panel === null ? "menu" : null);
            }}
          />
        </div>
      </div>
      {error !== null && (
        <p className="preview-error" role="alert">
          {error}
        </p>
      )}
      {(panel === "actions" || panel === "servers") && (
        <div className="browser-chrome-menu-dock">
          <div
            ref={popup}
            className="browser-popover browser-activity"
            role="region"
            aria-label={panel === "actions" ? "Browser actions" : "This session's servers"}
            onKeyDown={keys}
          >
            <header>
              <span>{panel === "actions" ? "Browser actions" : "This session's servers"}</span>
              <IconButton
                icon="x"
                label={
                  panel === "actions" ? "Close browser actions" : "Close this session's servers"
                }
                onClick={() => {
                  close();
                  trigger.current?.focus();
                }}
              />
            </header>
            {panel === "actions" ? actions : servers}
          </div>
        </div>
      )}
    </div>
  );
}
