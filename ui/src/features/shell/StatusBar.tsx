/**
 * The status bar: Vibe server state, the selected session's folder, the rate-limit warning, the context readout,
 * "N left running" while processes from deleted sessions still run, the vault access
 * switch, the Mistral key's blocking state, and the "Debug API on" indicator
 * (visible whenever the API runs).
 * Depends on: hooks, state/account, KeyNotice, LeftRunning. Called by: Shell.
 */
import { type ReactNode, useEffect, useState } from "react";

import { PathText } from "../../components/PathText";
import { contextReadout, rateLimitWarning, usageOf } from "../../state/account";
import { VaultLockControls } from "../secrets/VaultLockControls";
import { useApp } from "./AppContext";
import { useKeyReadiness } from "./KeyNotice";
import { LeftRunningButton } from "./LeftRunning";
import { useCoreState, useSessionSummaries, useShellState } from "./hooks";

function Server() {
  const server = useCoreState((s) => s.server);
  switch (server.phase) {
    case "running":
      return (
        // the item says "Vibe" (the server's own name, e.g. `vibe-app-server`, is a developer detail);
        // hovering gives the name and version Vibe reported.
        <span className="sb-server" title={`${server.name} ${server.version}`}>
          <span className="ok">■</span> Vibe
          <span className="sb-tail"> {server.version}</span>
        </span>
      );
    case "connecting":
    case "starting":
      return <span>Connecting to Vibe…</span>;
    case "restarting":
      return (
        <span>
          <span className="warn">■</span> Vibe restarting (attempt {server.attempt})
        </span>
      );
    case "dead":
    case "unavailable":
      return (
        <span>
          <span className="err">■</span> Vibe is not running
        </span>
      );
    case "notInstalled":
      return (
        <span>
          <span className="warn">■</span> Vibe is not installed yet
        </span>
      );
    case "stopped":
      return (
        <span>
          <span className="err">■</span> Vibe stopped
        </span>
      );
  }
}

/**
 * The open session's context readout (a short usage tracker): `Context 42%` and nothing
 * else (no token count, no bar). Shown only while the switch in Settings › Appearance is on and the numbers exist;
 * hovering explains it, a click opens Settings › Usage and plan at "This conversation".
 * (The per-minute rate-limit warning joins here when the key proxy reports headroom,.)
 */
function UsageReadout() {
  const { shell } = useApp();
  const show = useShellState((s) => s.showUsage);
  const selected = useShellState((s) => s.selected);
  const session = useCoreState((s) =>
    selected === null ? undefined : s.sessions[selected]?.session,
  );
  // the share of the model's declared window when its runtime says it (Vibe 2.26.1), else of the
  // compaction threshold.
  const runtime = useCoreState((s) =>
    selected === null ? undefined : s.sessions[selected]?.runtime,
  );
  if (!show || selected === null) return null;
  const readout = contextReadout(usageOf(session), runtime);
  if (readout === null) return null;
  const { percent } = readout;
  const explain = `${readout.explain} Click for Usage and plan.`;
  return (
    <button
      type="button"
      className="sb-usage"
      data-testid="statusbar-usage"
      title={explain}
      aria-label={`Context ${String(percent)}%: ${explain}`}
      onClick={() => {
        shell.actions.openSettings("usage", "session-usage");
      }}
    >
      Context {percent}%
    </button>
  );
}

/**
 * The rate-limit warning: "Rate limit: N% left this minute", only while Mistral's last
 * answer said less than 20 % of the minute's tokens or requests are left, and only for a minute after that answer
 * (a ticking clock hides it once it is stale). A click opens Settings › Usage and plan, where the full row is.
 */
function RateWarning() {
  const { shell } = useApp();
  const limits = useCoreState((s) => s.rateLimits);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (limits === null) return;
    // A new snapshot is newer than `now`, so it counts as fresh at once; the clock only has to notice it ageing.
    const timer = setInterval(() => {
      setNow(Date.now());
    }, 5_000);
    return () => {
      clearInterval(timer);
    };
  }, [limits]);
  const left = rateLimitWarning(limits, now);
  if (left === null) return null;
  return (
    <button
      type="button"
      className="sb-usage sb-rate"
      data-testid="statusbar-rate"
      title="Mistral's last answer said this much of the minute's allowance is left. It refills every minute."
      onClick={() => {
        shell.actions.openSettings("usage", "session-usage");
      }}
    >
      Rate limit: {left}%<span className="sb-tail"> left this minute</span>
    </button>
  );
}

/** The status bar. */
export function StatusBar({ children }: { children?: ReactNode }) {
  const key = useKeyReadiness();
  const debugOn = useCoreState((s) => s.debugApi.enabled);
  const selected = useShellState((s) => s.selected);
  const cwd = useSessionSummaries().find((s) => s.id === selected)?.cwd ?? null;
  return (
    <footer className="statusbar">
      <Server />
      {key !== null && (
        <span className="warn" data-testid="statusbar-key" title={key.message}>
          {key.status}
        </span>
      )}
      {/* The folder gives way (ellipsis) so the readout and the Debug API indicator stay visible at 390 px. */}
      {cwd !== null && <PathText path={cwd} className="sb-cwd" />}
      {children}
      <span className="push" />
      <LeftRunningButton />
      <RateWarning />
      <UsageReadout />
      <VaultLockControls mode="agent" display="status" />
      {debugOn && (
        <span className="warn" data-testid="statusbar-debug" title="Debug API on 127.0.0.1">
          ● Debug API<span className="sb-tail"> 127.0.0.1</span>
        </span>
      )}
    </footer>
  );
}
