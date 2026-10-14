/**
 * The small rows of the transcript: notice pills (Vibe's `notice` entries: plan review, agent
 * changed, scheduled loop, errors), checkpoint dividers ("Context compacted", "Session resumed",...),
 * the end-of-turn row and the generic row for entry types newer than this build.
 * The end-of-turn row shows "turn complete · 3.2s" with the stop reason, failed (Vibe's error text), and interrupted (a turn cut off
 * by a crash or a stop).
 * Error texts from the model provider go through `useErrorText`: a plain line, or the
 * provider's status code and message, as the Settings switch says.
 * Adapted from ShellX.
 * Depends on: model/entries, Time, state/errorDetail. Called by: RowView.
 */
import { type ReactElement, memo } from "react";

import { useErrorText } from "../../../state/errorDetail";
import { TEXT } from "../../../vocabulary";
import { type Obj, detailOf, entryId, entryType, num, obj, str } from "../model/entries";
import { formatElapsed } from "./Time";

/** A notice pill; `level` (`info`, `warning`, `error`) sets its tone. */
export const NoticeRow = memo(function NoticeRow({ entry }: { entry: Obj }): ReactElement {
  const level = str(entry["level"]) ?? "info";
  const message = useErrorText(str(entry["message"]) ?? "");
  return (
    <div
      className={`notice-pill ${level}`}
      data-entry-id={entryId(entry)}
      data-kind={str(detailOf(entry)["kind"]) ?? ""}
    >
      <span
        className={`px ${level === "error" ? "err" : level === "warning" ? "warn" : "ok"}`}
        aria-hidden="true"
      />
      {level === "error" ? <span className="sr">Error: </span> : null}
      {message}
    </div>
  );
});

const CHECKPOINT_TEXT: Record<string, string> = {
  compaction: "Context compacted",
  resume: "Session resumed",
  clear: "New conversation started",
  rewind: "Conversation rewound",
  agent_change: `${TEXT.accessMode} changed`,
};

/** A checkpoint divider. */
export const DividerRow = memo(function DividerRow({ entry }: { entry: Obj }): ReactElement {
  const kind = str(entry["kind"]) ?? "";
  const text = str(entry["message"]) || CHECKPOINT_TEXT[kind] || "Checkpoint";
  return (
    <div
      className="tx-divider"
      role="separator"
      aria-label={text}
      data-entry-id={entryId(entry)}
      data-kind={kind}
    >
      <span>{text}</span>
    </div>
  );
});

/** The end of a turn: complete (with its duration), failed or interrupted. */
export const TurnEndRow = memo(function TurnEndRow({ turn }: { turn: Obj }): ReactElement {
  const status = str(turn["status"]) ?? "";
  const error = useErrorText(str(obj(turn["error"])?.["message"]) ?? "The turn failed.");
  const started = num(turn["startedAt"]);
  const completed = num(turn["completedAt"]);
  const took =
    started !== null && completed !== null && completed > started
      ? formatElapsed(completed - started)
      : null;
  if (status === "failed") {
    return (
      <div className="turn-end fail" data-turn-id={str(turn["id"]) ?? ""}>
        <span className="px err" aria-hidden="true" />
        Turn failed: {error}
      </div>
    );
  }
  if (status === "interrupted") {
    return (
      <div className="turn-end int" data-turn-id={str(turn["id"]) ?? ""}>
        <span className="px warn" aria-hidden="true" />
        Interrupted{took === null ? "" : ` after ${took}`}: the turn did not finish.
      </div>
    );
  }
  const limit = turn["stopReason"] === "limit";
  return (
    <div className="turn-end ok" data-turn-id={str(turn["id"]) ?? ""}>
      <span className="px ok" aria-hidden="true" />
      {limit ? "Stopped at the turn limit" : "Turn complete"}
      {took === null ? "" : ` · ${took}`}
    </div>
  );
});

/** An entry of a type this build does not know: shown as text, never dropped, never a crash. */
export const OtherRow = memo(function OtherRow({ entry }: { entry: Obj }): ReactElement {
  const text = str(entry["message"]) ?? str(entry["text"]) ?? str(entry["title"]) ?? "";
  return (
    <div className="other-row" data-entry-id={entryId(entry)}>
      <span className="other-type">{entryType(entry) || "entry"}</span> {text}
    </div>
  );
});
