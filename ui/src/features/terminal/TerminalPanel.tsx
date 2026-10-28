/**
 * The side panel's Terminal tab: the shown session's terminal tabs, a toolbar (new,
 * restart, clear, copy, paste, close) and the active terminal's view. Switching sessions switches
 * terminals; a hidden session's terminals keep running in the core and buffer their output.
 * Depends on: store.ts, model.ts, TerminalView (lazy), features/shell (app context, shell state),
 * components/Icon, pages/point ("Point Vibe at this": the selection as a context chip), RowMenu (the right-click
 * menu: Point Vibe at this, Copy, Paste). Called by: register.tsx (the panel tab body).
 * Invariants: renders only; every action is a store method; on the New chat screen the terminals are its owner's
 * (`panelOwner`: the chosen folder, else the home folder). A tab's tooltip names the folder it started in; a terminal
 * that started before the chat in another folder than the session's says so.
 */
import { writeClipboard } from "../../state/clipboard";
import { Tabs, Tab } from "../../components/Tabs";
import { Suspense, lazy, useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { useStore } from "../../state/createStore";
import { NO_NEW_CHAT } from "../../state/newChatDraft";
import { detectMac } from "../../state/shortcuts";
import { createPortal } from "react-dom";
import { type MenuAt, RowMenu, menuGroups, useRowMenu } from "../../components/RowMenu";
import { terminalChip } from "../pages/model";
import { insertChip, pointTerminal } from "../pages/point";
import { readTerminalSelection } from "./selection";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";
import { panelOwner } from "../../state/shellState";
import { activeOf } from "./model";
import type { MountedTerminal } from "./session";
import type { Terminals } from "./store";
import { ViewBoundary } from "./ViewBoundary";

const TerminalView = lazy(() => import("./TerminalView"));
/** Copy/paste keys differ on macOS (session.ts uses the same rule). */
const MAC = detectMac();

/** A toolbar button. */
function Tool({
  testId,
  label,
  onClick,
  children,
}: {
  testId: string;
  label: string;
  onClick: () => void;
  children: string;
}) {
  return (
    <button
      data-testid={testId}
      type="button"
      className="vw-term-tool"
      title={label}
      aria-label={label}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** A terminal tab's tooltip: the folder it started in ("Home folder · ~/" for the home folder). */
function folderTitle(cwd: string, home: string | null): string {
  return home !== null && cwd === home ? "Home folder · ~/" : cwd;
}

/** The Terminal tab body. */
export function TerminalPanel({ terminals }: { terminals: Terminals }) {
  const app = useApp();
  const sessionId = useShellState(panelOwner);
  const [noticeText, setNoticeText] = useState<{ sessionId: string; text: string } | null>(null);
  const list = useStore(terminals.store, (s) =>
    sessionId === null ? undefined : s.bySession[sessionId],
  );
  const active = useStore(terminals.store, (s) =>
    sessionId === null ? null : activeOf(s, sessionId),
  );
  const error = useStore(terminals.store, (s) =>
    sessionId === null ? undefined : s.error[sessionId],
  );
  const home = useStore(app.newChat?.store ?? NO_NEW_CHAT, (s) => s.home);
  const sessionCwd = useCoreState((s) =>
    sessionId === null ? undefined : s.sessions[sessionId]?.session["cwd"],
  );
  const mounted = useRef<MountedTerminal | null>(null);
  const tabsRef = useRef<HTMLDivElement>(null);
  // Right click, the menu key or Shift F10 on the terminal: Point Vibe at this, Copy, Paste. The target is
  // the selection when the menu opened.
  const termMenu = useRowMenu<string>();

  useEffect(() => {
    if (sessionId !== null) void terminals.ensure(sessionId);
  }, [terminals, sessionId]);

  if (sessionId === null) {
    return (
      <div className="empty">
        Open a session to use its terminal (it starts in the session folder).
      </div>
    );
  }
  const tabs = list ?? [];
  const notice = (text: string) => {
    setNoticeText({ sessionId, text });
  };
  const leave = () => {
    tabsRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
  };
  const exited = active?.state.type === "exited" ? active.state : null;

  return (
    <div className="vw-term-panel" data-session-id={sessionId}>
      <div className="vw-term-bar">
        <Tabs
          data-testid="terminal.terminal-panel-vw-term-tabs"
          variant="strip"

          aria-label="Terminals"
          ref={tabsRef}
        >
          {tabs.map((t, index) => (
            <Tab
              data-testid="terminal.terminal-panel-select"
              data-item-id={t.terminalId}
              key={t.terminalId}
              type="button"

              aria-selected={t.terminalId === active?.terminalId}
              data-terminal-id={t.terminalId}
              data-state={t.state.type}
              title={folderTitle(t.cwd, home)}
              onClick={() => {
                terminals.select(sessionId, t.terminalId);
              }}
            >
              {`${String(index + 1)} ${t.shell.split(/[\\/]/).pop() ?? "shell"}`}
            </Tab>
          ))}
          <button
            data-testid="terminal.terminal-panel-open"
            type="button"
            className="ibtn vw-term-new"
            aria-label="New terminal"
            title="New terminal"
            onClick={() => void terminals.open(sessionId)}
          >
            <Icon name="plus" size={14} />
          </button>
        </Tabs>
        {active !== null && (
          <div className="vw-term-tools">
            <Tool
              testId="terminal.toolbar-copy"
              label={`Copy the selection (${MAC ? "Cmd+C" : "Ctrl+Shift+C"})`}
              onClick={() => void mounted.current?.copy()}
            >
              Copy
            </Tool>
            <Tool
              testId="terminal.toolbar-paste"
              label={`Paste (${MAC ? "Cmd+V" : "Ctrl+Shift+V"})`}
              onClick={() => void mounted.current?.paste()}
            >
              Paste
            </Tool>
            <Tool
              testId="terminal.toolbar-clear"
              label="Clear the view"
              onClick={() => mounted.current?.clear()}
            >
              Clear
            </Tool>
            <Tool
              testId="terminal.toolbar-action-1"
              label="Point Vibe at this: add the selected text to your next message"
              onClick={() => {
                const outcome = pointTerminal(app, sessionId);
                notice(
                  outcome.ok ? `Added to your message: ${outcome.chip.label}` : outcome.reason,
                );
              }}
            >
              Point Vibe at this
            </Tool>
            <Tool
              testId="terminal.toolbar-restart"
              label="Restart the shell (stops its processes)"
              onClick={() => void terminals.restart(sessionId, active.terminalId)}
            >
              Restart
            </Tool>
            <Tool
              testId="terminal.toolbar-close"
              label="Close the terminal (stops its processes)"
              onClick={() => void terminals.close(sessionId, active.terminalId)}
            >
              Close
            </Tool>
          </div>
        )}
      </div>
      {error !== undefined && (
        <div className="key-problem" role="alert">
          {error}
        </div>
      )}
      {noticeText?.sessionId === sessionId && (
        <div className="vw-term-exited" role="status">
          {noticeText.text}
        </div>
      )}
      {active !== null &&
        typeof sessionCwd === "string" &&
        sessionCwd !== "" &&
        active.cwd !== sessionCwd && (
          <div className="vw-term-exited" role="status">
            {`This terminal started before the chat, in ${active.cwd}.`}
          </div>
        )}
      {exited !== null && active !== null && (
        <div className="vw-term-exited" role="status">
          {`The shell exited${exited.exitCode === null ? "" : ` with code ${String(exited.exitCode)}`}${exited.signal === null ? "" : ` (${exited.signal})`}.`}
          <button
            data-testid="terminal.terminal-panel-restart"
            type="button"
            className="vw-term-tool"
            onClick={() => void terminals.restart(sessionId, active.terminalId)}
          >
            Restart
          </button>
        </div>
      )}
      <div
        data-testid="terminal.body"
        className="vw-term-body"
        onContextMenu={(event) => {
          if (active !== null) termMenu.triggers(readTerminalSelection()).onContextMenu(event);
        }}
        onKeyDown={(event) => {
          if (active !== null) termMenu.triggers(readTerminalSelection()).onKeyDown(event);
        }}
      >
        {termMenu.at !== null && (
          <TerminalMenu
            at={termMenu.at}
            onClose={termMenu.close}
            // The selection the menu names (the shell may redraw and clear the live one meanwhile).
            onPoint={(text) => {
              const outcome = insertChip(app, sessionId, terminalChip(text));
              notice(outcome.ok ? `Added to your message: ${outcome.chip.label}` : outcome.reason);
            }}
            onCopy={(text) => {
              void writeClipboard(text).then(
                () => {
                  notice("Copied the selected text.");
                },
                () => {
                  notice("The text could not be copied.");
                },
              );
            }}
            onPaste={() => void mounted.current?.paste()}
          />
        )}
        {active === null ? (
          <div className="empty">
            {list === undefined ? "Loading terminals…" : "No terminal open."}
          </div>
        ) : (
          <ViewBoundary key={active.terminalId}>
            <Suspense
              fallback={
                <div className="empty" aria-busy="true">
                  Loading the terminal…
                </div>
              }
            >
              <TerminalView
                key={active.terminalId}
                terminals={terminals}
                sessionId={sessionId}
                terminalId={active.terminalId}
                onMounted={(m) => {
                  mounted.current = m;
                }}
                notice={notice}
                leave={leave}
              />
            </Suspense>
          </ViewBoundary>
        )}
      </div>
    </div>
  );
}

/** The terminal's right-click menu: Point Vibe at this and Copy for a selection, Paste always. */
function TerminalMenu({
  at,
  onClose,
  onPoint,
  onCopy,
  onPaste,
}: {
  at: MenuAt<string>;
  onClose: () => void;
  onPoint: (text: string) => void;
  onCopy: (text: string) => void;
  onPaste: () => void;
}) {
  const selected = at.target.trim() !== "";
  return createPortal(
    <RowMenu
      testId="terminal.menu"
      at={at}
      label="Terminal actions"
      heading={selected ? terminalChip(at.target).label : "Terminal"}
      items={menuGroups(
        [
          selected && {
            id: "point",
            label: "Point Vibe at this",
            run: () => {
              onPoint(at.target);
            },
          },
        ],
        [
          selected && {
            id: "copy",
            label: "Copy",
            keys: MAC ? "Mod+C" : "Mod+Shift+C",
            run: () => {
              onCopy(at.target);
            },
          },
          { id: "paste", label: "Paste", keys: MAC ? "Mod+V" : "Mod+Shift+V", run: onPaste },
        ],
      )}
      onClose={onClose}
    />,
    document.body,
  );
}
