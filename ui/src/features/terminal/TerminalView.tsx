/**
 * The mount point of one terminal: a host element for xterm, mounted and disposed with the
 * component, restyled when the theme changes. All behaviour is in `session.ts`.
 * Loaded lazily (`TerminalPanel` uses `React.lazy`), so xterm is not in the first bundle (ShellX
 * `LazyTerminalView.tsx` did the same).
 * Depends on: session.ts, store.ts, features/shell (theme state). Called by: TerminalPanel.
 * Invariants: one mounted xterm per terminal id at a time; unmounting detaches the output stream
 * but never stops the terminal (it keeps running and buffering in the core).
 */
import { useEffect, useRef } from "react";

import { useShellState } from "../shell/hooks";
import { type MountedTerminal, mountTerminal } from "./session";
import type { Terminals } from "./store";
import { detectMac } from "../../state/shortcuts";

/** The key that moves focus out of the terminal (session.ts). */
const LEAVE_KEYS = detectMac() ? "Cmd+`" : "Ctrl+`";

/** Props of the view. */
export interface TerminalViewProps {
  terminals: Terminals;
  sessionId: string;
  terminalId: string;
  /** Receives the mounted terminal (toolbar actions), `null` on unmount. */
  onMounted: (terminal: MountedTerminal | null) => void;
  notice: (text: string) => void;
  /** Ctrl+` (Cmd+`) inside the terminal: where focus goes. */
  leave: () => void;
}

/** One terminal's xterm view. */
export default function TerminalView({
  terminals,
  sessionId,
  terminalId,
  onMounted,
  notice,
  leave,
}: TerminalViewProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const mountedRef = useRef<MountedTerminal | null>(null);
  const theme = useShellState((s) => s.theme);
  // The latest callbacks without remounting xterm when the parent re-renders.
  const callbacks = useRef({ onMounted, notice, leave });
  useEffect(() => {
    callbacks.current = { onMounted, notice, leave };
  });

  useEffect(() => {
    const host = hostRef.current;
    if (host === null) return;
    const mounted = mountTerminal(host, {
      terminals,
      sessionId,
      terminalId,
      notice: (text) => {
        callbacks.current.notice(text);
      },
      leave: () => {
        callbacks.current.leave();
      },
    });
    mountedRef.current = mounted;
    callbacks.current.onMounted(mounted);
    mounted.focus();
    return () => {
      callbacks.current.onMounted(null);
      mountedRef.current = null;
      mounted.dispose();
    };
  }, [terminals, sessionId, terminalId]);

  useEffect(() => {
    // The shell sets `data-theme` in its own effect, which runs after this child's effect: read the
    // tokens on the next tick.
    const timer = setTimeout(() => mountedRef.current?.restyle(), 0);
    return () => {
      clearTimeout(timer);
    };
  }, [theme]);

  return (
    <div
      ref={hostRef}
      className="vw-terminal"
      data-toast-avoid="terminal"
      data-terminal-id={terminalId}
      role="region"
      aria-label={`Terminal (${LEAVE_KEYS} leaves it)`}
    />
  );
}
