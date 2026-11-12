/**
 * The Browser tab's Capture row: a "Capture" button opening a menu of what to save from the page shown
 * (design system, structure, media, the whole page in one or every format, everything, a before / after shot), and
 * one status line: running, the last result with "Show in folder", or the error. The page area is a native webview
 * painted above HTML; the shared Browser overlay host parks it while menus are open.
 * Depends on: capture.ts, components/Icon, useBrowserPopup, state/createStore. Called by: PreviewTab.
 * Invariants: keyboard reachable (Enter/Space open, arrows move, Escape closes and returns focus); the menu never
 * covers an action the user did not open; texts are Vibe Work's own (the page's words are never shown here).
 */
import type { CaptureBlock } from "../../protocol/core";
import { Button } from "../../components/Controls";
import {
  Fragment,
  type KeyboardEvent,
  type ReactElement,
  useCallback,
  useRef,
  useState,
} from "react";

import { Icon } from "../../components/Icon";
import { useStore } from "../../state/createStore";
import { CAPTURE_KINDS, type Capture } from "./capture";
import { useBrowserPopup } from "./useBrowserPopup";
import { closeBrowserOverlaysAndWait } from "./browserOverlays";

/** Short window labels use typed reasons; detailed guidance stays in the capture result. */
const BLOCK_LABELS: Record<CaptureBlock, string> = {
  windowHidden: "Window hidden",
  anotherSession: "Another chat",
  anotherPage: "Another page",
  noChatSelected: "No chat selected",
  browserPanelClosed: "Browser panel closed",
  browserPaused: "Browser paused",
  appPreviewNotSelected: "App preview not selected",
  webTabNotSelected: "Web tab not selected",
  serverStopped: "Server stopped",
  appPreviewNotShown: "App preview not shown",
  webTabNotShown: "Web tab not shown",
  browserPageNotShown: "Browser page not shown",
};

/** An unknown failure keeps its complete text. */
export function captureErrorLabel(error: string, block?: CaptureBlock): string {
  return block === undefined ? error : BLOCK_LABELS[block];
}

/** The row for `sessionId`. `reveal` opens a folder in the system's file manager. */
export function CaptureMenu({
  capture,
  sessionId,
  tabId,
  reveal,
  compact = false,
  statusOnly = false,
}: {
  capture: Capture;
  sessionId: string;
  tabId?: string;
  reveal: (path: string) => void;
  /** Overflow entry: capture choices expand inside the floating parent menu. */
  compact?: boolean;
  /** Keep the existing result row visible below the toolbar after its overflow closes. */
  statusOnly?: boolean;
}): ReactElement {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const dismiss = useCallback(() => {
    setOpen(false);
  }, []);
  const menu = useBrowserPopup("capture-menu", open, button, dismiss);
  const busy = useStore(capture.store, (s) => s.busy[sessionId]);
  const last = useStore(capture.store, (s) => s.last[sessionId]);
  const error = useStore(capture.store, (s) => s.error[sessionId] ?? null);
  const block = useStore(capture.store, (s) => s.block[sessionId]);

  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  const choose = (id: string) => {
    close();
    void closeBrowserOverlaysAndWait()
      .then(() => capture.capture(sessionId, id, {}, tabId))
      .then(() => {
        if (compact)
          document.querySelector<HTMLButtonElement>('[aria-label="Browser options"]')?.focus();
        else button.current?.focus();
      })
      .catch(() => undefined); // The preview controller reports a failed native restoration.
  };
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(
      menu.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [],
    );
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      items[(at + step + items.length) % items.length]?.focus();
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      items[event.key === "Home" ? 0 : items.length - 1]?.focus();
    }
  };

  if (statusOnly && busy === undefined && last === undefined && error === null) return <></>;
  return (
    <div className="preview-capture">
      {!statusOnly && (
        <Button
          data-testid="preview.capture-menu-set-open"
          ref={button}
          type="button"
          appearance={compact ? "browser-menu" : "preview"}
          placement="browser-capture"
          role={compact ? "menuitem" : undefined}
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={busy !== undefined}
          title="Save the page's design system, structure, media or the whole page into this chat's folder"
          onClick={() => {
            setOpen((o) => !o);
          }}
        >
          <Icon name="download" size={14} />
          Capture
        </Button>
      )}
      {!compact && (
        <>
          <span className="preview-capture-status" role="status" aria-live="polite">
            {busy !== undefined &&
              (busy === "PDF" || busy === "one-file web page"
                ? `Saving ${busy}…`
                : `Capturing: ${busy}…`)}
            {busy === undefined && error === null && last !== undefined && (
              <>
                {last.kind === "pdf" || last.kind === "single_html" ? (
                  <>
                    Saved {last.fileNames.join(", ")} to <code>{last.folder}/</code>
                  </>
                ) : (
                  <>
                    Saved {String(last.files)} file{last.files === 1 ? "" : "s"} to{" "}
                    <code>{last.folder}</code>
                    {last.fileNames.length > 0 && (
                      <>
                        : <code>{last.fileNames.join(", ")}</code>
                      </>
                    )}
                  </>
                )}
                {last.keptAsLinks > 0 &&
                  ` · ${String(last.keptAsLinks)} picture${last.keptAsLinks === 1 ? "" : "s"} kept as links`}
                {last.skipped > 0 &&
                  `, ${String(last.skipped)} part${last.skipped === 1 ? "" : "s"} skipped (see capture.json)`}
                <button
                  data-testid="preview.capture-menu-reveal"
                  type="button"
                  className="preview-capture-reveal"
                  onClick={() => {
                    reveal(last.path);
                  }}
                >
                  Show in folder
                </button>
                <button
                  data-testid="preview.capture-menu-dismiss"
                  type="button"
                  className="ibtn"
                  aria-label="Dismiss"
                  title="Dismiss"
                  onClick={() => {
                    capture.dismiss(sessionId);
                  }}
                >
                  <Icon name="x" size={12} />
                </button>
              </>
            )}
          </span>
          {error !== null && busy === undefined && (
            <p className="key-problem preview-error" role="alert">
              {captureErrorLabel(error, block)}
            </p>
          )}
        </>
      )}
      {open && (
        <div
          data-testid="preview.capture-menu-div"
          ref={menu}
          className={
            compact ? "preview-capture-menu browser-capture-inline" : "preview-capture-menu"
          }
          role="menu"
          aria-label="Capture"
          onKeyDown={keys}
        >
          {CAPTURE_KINDS.map((kind) => (
            <Fragment key={kind.id}>
              {kind.caption !== undefined && (
                <p className="preview-capture-note" role="presentation">
                  {kind.caption}
                </p>
              )}
              <button
                data-testid="preview.capture-menu-choose"
                data-item-id={kind.id}
                type="button"
                role="menuitem"
                className="preview-capture-item"
                data-group={kind.group === true}
                data-capture={kind.id}
                onClick={() => {
                  choose(kind.id);
                }}
              >
                <span className="preview-capture-label">{kind.label}</span>{" "}
                <span className="preview-capture-hint">{kind.hint}</span>
              </button>
            </Fragment>
          ))}
          <p className="preview-capture-note">
            Saved in this chat&apos;s folder under capture/. Captured logos, images and fonts are
            for reference: the site&apos;s licence applies.
          </p>
        </div>
      )}
    </div>
  );
}
