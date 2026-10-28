/**
 * The collapsed steps group of a turn: a header with the live count and Vibe's newest
 * `statusText` while the turn works ("Running command · 3 steps"), and, when opened, the items in
 * order: effect steps, "Thinking" blocks, the assistant's narration between steps and one-line audit
 * rows of resolved callbacks. The thinking toggle expands the group's reasoning.
 * Adapted from ShellX.
 * Depends on: model (build, entries, callbacks), StepItem, Markdown, the transcript store.
 * Called by: RowView.
 * Invariants: open state per group key / reasoning id in the store; reasoning and narration are
 * shown only because Vibe sent them (a session without thinking has no reasoning entries).
 */
import { type ReactElement, memo } from "react";
import { AuditText } from "../../ask/AuditRow";
import { approvalSummary, auditLine, neverText } from "../../ask/model";
import { Walker } from "../../../components/Walker";

import { Markdown } from "../../../components/markdown/Markdown";
import { useApp } from "../../shell/AppContext";
import { useCoreState, useShellState } from "../../shell/hooks";
import { useTranscriptUi } from "../hooks";
import type { Row, StepItem as Item } from "../model/build";
import { auditText } from "../model/callbacks";
import {
  type Obj,
  arr,
  createdAt,
  entryId,
  inProgress,
  messageText,
  num,
  obj,
  str,
} from "../model/entries";
import { StepItem } from "./StepItem";

type StepsRow = Extract<Row, { kind: "steps" }>;

const Reasoning = memo(function Reasoning({ entry }: { entry: Obj }): ReactElement {
  const { transcript } = useApp();
  const key = `think:${entryId(entry)}`;
  const expandedByDefault = useShellState((s) => s.showThinkingExpanded);
  const open = useTranscriptUi((s) => s.open[key] ?? expandedByDefault);
  const text =
    str(entry["text"]) ||
    arr(entry["summary"])
      .filter((x) => typeof x === "string")
      .join("\n");
  const seconds = Math.round(((num(entry["updatedAt"]) ?? 0) - createdAt(entry)) / 1000);
  const label = inProgress(entry)
    ? "Thinking…"
    : seconds >= 1
      ? `Thought for ${String(seconds)} s`
      : "Thinking";
  const actionLabel = open ? "Hide thinking" : "Show thinking";
  return (
    <div className="think">
      <button
        data-testid="transcript.steps-group-toggle"
        type="button"
        className="thought"
        aria-expanded={open}
        aria-label={actionLabel}
        title={actionLabel}
        onClick={() => {
          transcript.toggle(key, !open);
        }}
      >
        <span className="caret" aria-hidden="true" />
        {label}
      </button>
      {open ? <div className="thought-body">{text}</div> : null}
    </div>
  );
});

const Narration = memo(function Narration({ entry }: { entry: Obj }): ReactElement {
  const { transcript } = useApp();
  const text = messageText(entry);
  return (
    <div className="narr" data-entry-id={entryId(entry)}>
      <Markdown
        source={text}
        streaming={inProgress(entry)}
        lineBreaks
        onOpenLink={(url) => {
          transcript.askLink(url);
        }}
      />
    </div>
  );
});

/**
 * A resolved callback's audit row: the core's record when it is still held (this run, newest 500), else
 * the words Vibe's own entry allows (an older session).
 */
function Audit({ entry, related }: { entry: Obj; related: Obj | null }): ReactElement {
  const callbackId = str(entry["callbackId"]) ?? "";
  const via = useTranscriptUi((s) => s.answered[callbackId]);
  const always = useTranscriptUi((s) => s.always[callbackId]);
  const origin = useCoreState((s) => s.answeredBy[callbackId]);
  const record = useCoreState((s) => s.askRecords[`callback:${callbackId}`]);
  const summary = approvalSummary(entry);
  const question = obj(entry["detail"])?.["kind"] === "user_input";
  // Question answers come from Vibe’s entry.
  const fromRecord =
    record === undefined || question ? null : auditLine(record, summary, via, always);
  const text = auditText(entry, related, via, origin);
  const line = fromRecord ?? {
    square: /^(Said No|Closed|No answer|Stopped|Question skipped)/.test(text) ? "warn" : "ok",
    text,
  };
  return <AuditText line={line} callbackId={callbackId} entryId={entryId(entry)} />;
}

/**
 * The audit row of an explicit Never under the step it refused: no card was shown, the step failed, and the
 * core recorded `never` for one of Vibe Work's own tools in this session while the step ran. The core's record
 * carries no step id, so the step is found by its tool and time (the refusal happens inside the call).
 */
function NeverAudit({ entry }: { entry: Obj }): ReactElement | null {
  const tool = str(obj(entry["detail"])?.["toolName"]) ?? "";
  const failed = str(obj(entry["state"])?.["status"]) === "failed";
  const session = useShellState((s) => s.selected);
  const from = createdAt(entry);
  const to = (num(entry["updatedAt"]) ?? from) + NEVER_SLACK_MS;
  const record = useCoreState((s) =>
    !failed || !tool.startsWith("mcp_vw.")
      ? undefined
      : Object.values(s.askRecords).find(
          (r) =>
            r.outcome.type === "never" &&
            r.ask.session === session &&
            r.closedAt >= from &&
            r.closedAt <= to,
        ),
  );
  if (record === undefined) return null;
  return <AuditText line={{ square: "err", text: neverText(record.ask.title, record.ask.kind) }} />;
}

/** How long after a step's last update a Never record still belongs to it (ms). */
const NEVER_SLACK_MS = 5000;

function ItemView({ item }: { item: Item }): ReactElement {
  switch (item.kind) {
    case "effect":
      return (
        <>
          <StepItem entry={item.entry} />
          <NeverAudit entry={item.entry} />
        </>
      );
    case "reasoning":
      return <Reasoning entry={item.entry} />;
    case "narration":
      return <Narration entry={item.entry} />;
    case "audit":
      return <Audit entry={item.entry} related={item.related} />;
  }
}

const plural = (n: number, one: string, many: string): string =>
  n === 1 ? `1 ${one}` : `${String(n)} ${many}`;

function headerText(row: StepsRow): string {
  const count = plural(row.effects, "step", "steps");
  if (row.live) {
    return row.effects > 0
      ? `${row.statusText || "Working"} · ${count}`
      : row.statusText || "Working";
  }
  if (row.effects > 0) return row.failed > 0 ? `${count} · ${String(row.failed)} failed` : count;
  // A group without effects: name what it holds (thinking, answered requests, notes).
  const n = (kind: string) => row.items.filter((i) => i.kind === kind).length;
  return [
    n("reasoning") > 0 ? "Thinking" : "",
    n("audit") > 0 ? plural(n("audit"), "answered request", "answered requests") : "",
    n("narration") > 0 ? plural(n("narration"), "note", "notes") : "",
  ]
    .filter((part) => part !== "")
    .join(" · ");
}

/** Renders a steps group. */
export function StepsGroup({ row }: { row: StepsRow }): ReactElement {
  const { transcript } = useApp();
  const expandedByDefault = useShellState((s) => s.showThinkingExpanded);
  const hasThinking = row.items.some((item) => item.kind === "reasoning");
  const open = useTranscriptUi((s) => s.open[row.key] ?? (expandedByDefault && hasThinking));
  const label = open ? "Hide steps and notes" : "Show steps and notes";
  return (
    <div
      className={`steps${row.live ? " live" : ""}${row.failed > 0 ? " has-fail" : ""}`}
      data-row={row.key}
    >
      <button
        data-testid="transcript.steps-group-toggle-2"
        type="button"
        className="steps-h"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={() => {
          transcript.toggle(row.key, !open);
        }}
      >
        <span className="caret" aria-hidden="true" />
        {row.live ? <Walker /> : null}
        <span className="steps-label">{headerText(row)}</span>
      </button>
      {open ? (
        <div className="lane">
          {row.items.map((item) => (
            <ItemView key={`${item.kind}:${entryId(item.entry)}`} item={item} />
          ))}
        </div>
      ) : null}
    </div>
  );
}
