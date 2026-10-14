/**
 * The browser's action log: what the agent (and a Debug API client) did in this session's browser,
 * newest first, with the outcome. The user's own clicks and typing in the page are never listed (only `browser_act`
 * calls are). Targets and details are page text: shown as text only.
 * Depends on: browserModel.ts, protocol types. Called by: PreviewTab.
 */
import type { ReactElement } from "react";

import type { LogEntry } from "../../protocol/core";
import { actorText, outcomeText } from "./browserModel";

const SHOWN = 12;

/** A collapsed list of the recent actions; nothing before the first one. */
export function ActionLog({
  log,
  expanded = false,
}: {
  log: readonly LogEntry[];
  expanded?: boolean;
}): ReactElement | null {
  if (log.length === 0) return null;
  const recent = [...log].reverse().slice(0, SHOWN);
  return (
    <details className="browser-log" open={expanded}>
      <summary>
        Browser actions ({String(log.length)}) · last: {recent[0]?.action}{" "}
        {recent[0] ? outcomeText(recent[0]) : ""}
      </summary>
      <ol className="browser-log-list">
        {recent.map((entry) => (
          <li
            key={entry.seq}
            className="browser-log-row"
            data-outcome={entry.outcome}
            data-by={entry.by}
          >
            <span className="browser-log-who">{actorText(entry)}</span>
            <span className="browser-log-action">{entry.action}</span>
            <span className="browser-log-target">{entry.target}</span>
            <span className="browser-log-outcome">{outcomeText(entry)}</span>
            {entry.detail !== "" && <span className="browser-log-detail">{entry.detail}</span>}
          </li>
        ))}
      </ol>
    </details>
  );
}
