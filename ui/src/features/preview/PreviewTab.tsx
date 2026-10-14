/**
 * Browser tab: one compact toolbar with overflow for captures, annotations, servers and activity receipts.
 * Empty server guidance lives in the page area; loaded pages never show a stale no-server message.
 * Loads the shown session's list and browser state on show; live updates come from the stores (`previewChanged`,
 * `browserChanged`). Queued and paused engines keep a visible message in the page area.
 * Depends on: store.ts, browser.ts, Toolbar, AddressBar, AgentStrip, ActionLog, Annotations, Servers, Viewport, shell
 * hooks, AppContext. Called by: register.tsx (the panel body).
 */
import { type ReactElement, useEffect, useState } from "react";

import { useStore } from "../../state/createStore";
import { pointPreview } from "../pages/point";
import { useApp } from "../shell/AppContext";
import { KeyNotice, useKeyReadiness } from "../shell/KeyNotice";
import { useShellState } from "../shell/hooks";
import { ActionLog } from "./ActionLog";
import { DelegationReceipts } from "./DelegationReceipts";
import { AddressBar } from "./AddressBar";
import { BrowserOverlayLayer } from "./BrowserOverlayLayer";
import { AgentStrip } from "./AgentStrip";
import type { Capture } from "./capture";
import { CaptureMenu } from "./CaptureMenu";
import { Annotations } from "./Annotations";
import type { Browser } from "./browser";
import { isLocalPage } from "./browserModel";
import { PageTabs } from "./PageTabs";
import { PreviewNotes, Servers } from "./Servers";
import { SaveVault } from "./SaveVault";
import type { Previews } from "./store";
import { Toolbar } from "./Toolbar";
import { Viewport } from "./Viewport";

/** The tab. */
export function PreviewTab({
  previews,
  browser,
  capture,
}: {
  previews: Previews;
  browser: Browser;
  /** The Capture menu. */
  capture: Capture;
}): ReactElement {
  const app = useApp();
  const vaultBlocked = useKeyReadiness("vault") !== null;
  const sessionId = useShellState((s) => s.selected);
  const [pointNotice, setPointNotice] = useState<{ sessionId: string; text: string } | null>(null);
  const [saveNotice, setSaveNotice] = useState<{ sessionId: string; text: string } | null>(null);
  const list = useStore(previews.store, (s) =>
    sessionId === null ? undefined : s.lists[sessionId],
  );
  const busy = useStore(previews.store, (s) =>
    sessionId === null ? false : s.busy[sessionId] === true,
  );
  const error = useStore(previews.store, (s) =>
    sessionId === null ? null : (s.error[sessionId] ?? null),
  );
  const agent = useStore(browser.store, (s) =>
    sessionId === null ? undefined : s.states[sessionId],
  );
  const tabs = useStore(browser.store, (s) => (sessionId === null ? undefined : s.tabs[sessionId]));
  const agentError = useStore(browser.store, (s) =>
    sessionId === null ? null : (s.error[sessionId] ?? null),
  );
  const picking = useStore(browser.store, (s) =>
    sessionId === null ? undefined : s.picking[sessionId],
  );
  useEffect(() => {
    if (sessionId === null) return;
    void previews.load(sessionId);
    void browser.load(sessionId);
    void browser.loadTabs(sessionId);
  }, [previews, browser, sessionId]);

  if (sessionId === null) return <div className="empty">Choose a session to use its browser.</div>;
  const selected = tabs?.find((tab) => tab.active);
  const webTabId = selected?.kind === "web" ? selected.pageKey : undefined;
  const selectedTabId = selected?.pageKey ?? sessionId;
  const preview = webTabId ? (selected?.preview ?? null) : (list?.preview ?? null);
  const open = preview !== null && preview.url !== null;
  const waiting = preview?.waitingForBrowser === true;
  const paused = open && preview.state === "suspended" && !waiting;
  const stopped = !webTabId && preview?.state === "stopped";
  const local = !webTabId && open && isLocalPage(preview.url);
  const shownError = error ?? agentError;
  const servers =
    list === undefined ? (
      <div className="empty">Reading this session's servers…</div>
    ) : (
      <Servers sessionId={sessionId} list={list} previews={previews} busy={busy} />
    );
  return (
    <div className="preview-tab" data-open={open}>
      <BrowserOverlayLayer
        previews={previews}
        sessionId={sessionId}
        {...(webTabId ? { tabId: webTabId } : {})}
        pageUrl={preview?.url ?? null}
      />
      <PageTabs
        key={`tabs-${sessionId}`}
        browser={browser}
        sessionId={sessionId}
        tabs={tabs ?? []}
        busy={busy}
      />
      {agent?.pendingNativeClick && (
        <p className="preview-focus" role="status">
          {agent.pendingNativeClick}
        </p>
      )}
      <Toolbar
        key={`toolbar-${sessionId}`}
        previews={previews}
        browser={browser}
        {...(webTabId ? { webTabId } : {})}
        preview={preview}
        sessionId={sessionId}
        busy={busy}
        error={shownError}
        address={
          <AddressBar
            key={selectedTabId}
            previews={previews}
            browser={browser}
            {...(webTabId ? { webTabId } : {})}
            sessionId={sessionId}
            url={preview?.url ?? ""}
            title={preview?.title ?? null}
            loading={preview?.loading ?? false}
            busy={busy}
          />
        }
        activity={
          <AgentStrip
            state={agent}
            onStop={() => void browser.stopAgent(sessionId)}
            onResume={() => void browser.resumeAgent(sessionId)}
          />
        }
        save={(collapsed, close, visible) =>
          open && (
            <SaveVault
              browser={browser}
              sessionId={sessionId}
              tabId={selectedTabId}
              disabled={busy}
              compact
              collapsed={collapsed}
              onChoose={close}
              onNotice={(text) => {
                setSaveNotice({ sessionId, text });
              }}
              visible={visible}
            />
          )
        }
        capture={
          open && (
            <CaptureMenu
              capture={capture}
              sessionId={sessionId}
              tabId={selectedTabId}
              compact
              reveal={(path) => void app.sessions?.revealPath(path)}
            />
          )
        }
        annotate={
          local && picking === undefined
            ? (mode) => {
                void browser.annotate(sessionId, mode);
              }
            : null
        }
        actions={
          <>
            {open && (
              <div role="menu" aria-label="Capture actions">
                <CaptureMenu
                  capture={capture}
                  sessionId={sessionId}
                  tabId={selectedTabId}
                  compact
                  reveal={(path) => void app.sessions?.revealPath(path)}
                />
              </div>
            )}
            <ActionLog log={agent?.log ?? []} expanded />
            <DelegationReceipts receipts={agent?.receipts ?? []} expanded />
            {(agent?.log.length ?? 0) === 0 && (agent?.receipts.length ?? 0) === 0 && (
              <p className="empty">No browser actions yet.</p>
            )}
            {open && vaultBlocked && (
              <KeyNotice
                requirement="vault"
                message="Saving page values uses your vault to keep them encrypted."
              />
            )}
          </>
        }
        servers={
          webTabId || list === undefined ? null : (
            <Servers
              sessionId={sessionId}
              list={list}
              previews={previews}
              busy={busy}
              emptyState={false}
            />
          )
        }
        openExternal={(url) => {
          app.transcript.askLink(url);
        }}
        point={
          local
            ? () => {
                setPointNotice({
                  sessionId,
                  text: "Select text or click an element in the preview (Escape cancels).",
                });
                void pointPreview(app, sessionId).then((outcome) => {
                  setPointNotice({
                    sessionId,
                    text: outcome.ok
                      ? `Added to your message: ${outcome.chip.label}`
                      : outcome.reason,
                  });
                });
              }
            : null
        }
      />
      {pointNotice?.sessionId === sessionId && (
        <p className="preview-focus" role="status">
          {pointNotice.text}
        </p>
      )}
      {saveNotice?.sessionId === sessionId && (
        <p className="preview-focus" role="status">
          {saveNotice.text}
        </p>
      )}
      {local && <Annotations browser={browser} sessionId={sessionId} controls={false} />}
      {open && <PreviewNotes preview={preview} showRefused={!stopped} />}
      <Viewport
        previews={previews}
        sessionId={sessionId}
        {...(webTabId ? { tabId: webTabId } : {})}
        live={open && !waiting && !stopped}
        waiting={waiting}
        paused={paused}
        stopped={stopped ? preview.url : null}
        snapshot={selected?.snapshot ?? null}
        onReload={
          webTabId
            ? () => void browser.webAction(sessionId, webTabId, "reload")
            : () => void previews.reload(sessionId)
        }
      >
        {webTabId ? null : servers}
      </Viewport>
    </div>
  );
}
