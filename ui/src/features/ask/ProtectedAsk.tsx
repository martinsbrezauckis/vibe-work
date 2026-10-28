/**
 * A protected change (a change Vibe proposes to its own folder) as the one AskCard: header
 * "Allow this change to Vibe's folder?", the reason, the file list with Add/Edit/Remove badges and diffs, Allow once
 * 1 / No 2 and the note. "Applying…" while the core applies it; afterwards the card becomes its audit row ("Allowed
 * once: change to Vibe's folder (3 files) · by you") with "Show the changes" under it, which opens the same files
 * and diffs read only. The review record on disk is unchanged.
 * Depends on: AskCard, AuditText, model.ts, the protected-change model, DiffView, Badge, the stores.
 * Called by: RowView. Invariant: only a pending review offers answers; the first offered answer receives directed focus.
 */
import { type ReactElement } from "react";

import { Badge } from "../../components/Badge";
import { Button } from "../../components/Controls";
import type { AskRecord, ProtectedChangeView } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { useHeldBy } from "../sessions/SessionHeldNote";
import { useTranscriptUi } from "../transcript/hooks";
import {
  protectedDiff,
  protectedSize,
  protectedStateText,
} from "../transcript/model/protectedChange";
import { DiffView } from "../transcript/rows/DiffView";
import { AskCard } from "./AskCard";
import { AuditText } from "./AuditRow";
import { type AuditLine, auditLine, buttonsFor, whoAnswered } from "./model";
import { useCardClose } from "./useCardClose";
import { useWho } from "./useWho";

/** "change to Vibe's folder (3 files)": the summary the audit row names. */
function protectedSummary(p: ProtectedChangeView): string {
  const n = p.files.length;
  return `change to Vibe's folder (${String(n)} ${n === 1 ? "file" : "files"})`;
}

/** The audit row of a decided review, from the review's own status and the core's record. */
function protectedAudit(
  p: ProtectedChangeView,
  record: AskRecord | undefined,
  via: "window" | "uiControl" | undefined,
): AuditLine {
  const summary = protectedSummary(p);
  switch (p.status) {
    case "applied":
      return {
        square: "ok",
        text: `Allowed once: ${summary} · ${whoAnswered(record, via)}${p.files.some((f) => f.path.startsWith("skills/")) ? ". New chats use it." : ""}`,
      };
    case "declined":
      if (p.stopped) return { square: "warn", text: `Stopped: ${summary}` };
      return (
        auditLine(record, summary, via) ?? {
          square: "warn",
          text: `Said No: ${summary} · ${whoAnswered(record, via)}`,
        }
      );
    case "failed":
      return { square: "err", text: `Not applied: ${summary}. ${protectedStateText(p)}` };
    default:
      return { square: "warn", text: `${summary}: ${protectedStateText(p)}` };
  }
}

/** The review's files with their badges and literal diffs (three, then "Show all {n} files"). */
function ProtectedFiles({ p, full }: { p: ProtectedChangeView; full: boolean }): ReactElement {
  const { transcript } = useApp();
  const key = `protected:${p.id}`;
  return (
    <>
      {p.files.map((file, index) => (
        <div key={file.path} className="protected-file">
          <div className="fh">
            <Badge>{file.kind === "add" ? "Add" : file.kind === "edit" ? "Edit" : "Remove"}</Badge>
            <span className="fh-path">{file.path}</span>
            <span className="fh-stat">{protectedSize(file)}</span>
          </div>
          {file.diff === null || (!full && index >= 3) ? null : (
            <DiffView diff={protectedDiff(file)} id={`${key}:${file.path}`} />
          )}
          {file.diffTruncated ? <p className="card-state">Saved diff truncated.</p> : null}
        </div>
      ))}
      {!full && p.files.length > 3 ? (
        <Button
          data-testid="transcript.protected-change-show-all"
          type="button"
          appearance="expander"
          onClick={() => {
            transcript.toggle(key, true);
          }}
        >
          Show all {p.files.length} files
        </Button>
      ) : null}
    </>
  );
}

/**
 * A decided review: its audit row, and under it "Show the changes" that opens the files and diffs the user
 * answered (read only), so what was allowed or refused stays visible in the transcript.
 */
function DecidedProtected({ p, line }: { p: ProtectedChangeView; line: AuditLine }): ReactElement {
  const { transcript } = useApp();
  const key = `protected:${p.id}`;
  const shown = useTranscriptUi((s) => s.open[`${key}:changes`] === true);
  const full = useTranscriptUi((s) => s.open[key] === true);
  return (
    <>
      <AuditText line={line} entryId={key} />
      {p.files.length === 0 ? null : (
        <Button
          data-testid="transcript.protected-change-changes"
          data-protected-changes=""
          type="button"
          appearance="expander"
          aria-expanded={shown}
          onClick={() => {
            transcript.toggle(`${key}:changes`, !shown);
          }}
        >
          {shown ? "Hide the changes" : "Show the changes"}
        </Button>
      )}
      {shown ? (
        <div className="protected-change-files">
          <ProtectedFiles p={p} full={full} />
        </div>
      ) : null}
    </>
  );
}

/** Renders one captured review. */
export function ProtectedAsk({
  proposal: p,
  sessionId,
}: {
  proposal: ProtectedChangeView;
  sessionId: string;
}): ReactElement {
  const { transcript } = useApp();
  const key = `protected:${p.id}`;
  const full = useTranscriptUi((s) => s.open[key] === true);
  const sending = useTranscriptUi((s) => s.sending[key] === true);
  const problem = useTranscriptUi((s) => s.problems[key]);
  const record = useCoreState((s) => s.askRecords[`proposal:${p.id}`]);
  const held = useHeldBy(sessionId) !== null;
  const who = useWho("agent", sessionId, sessionId);
  const pending = p.status === "pending";
  const applying = p.status === "applying" || sending;
  const closedLine = !pending && !applying ? protectedAudit(p, record, undefined) : null;
  const focusInside = useCardClose(closedLine !== null, sessionId, closedLine?.text ?? null);
  if (closedLine !== null) return <DecidedProtected p={p} line={closedLine} />;
  const disabled = held || applying;
  return (
    <AskCard
      id={key}
      className="protected-change"
      question="Allow this change to Vibe's folder?"
      who={who.text}
      whoTitle={who.title}
      reason={`Reason: ${p.reason}`}
      buttons={buttonsFor(["once", "no"], null)}
      disabled={disabled}
      onChoose={(choice) => {
        void transcript.decideProtectedChange(sessionId, p.id, choice === "once", null);
      }}
      state={applying ? "Applying…" : (problem?.message ?? "")}
      stateRole={problem === undefined ? "status" : "alert"}
      decisionOf={(choice) => (choice === "once" ? "apply" : "decline")}
      onFocusInside={focusInside}
      data={{ "data-entry-id": key }}
    >
      {p.note ? <p className="reason protected-note">{p.note}</p> : null}
      <ProtectedFiles p={p} full={full} />
      {pending ? <span className="sr">Vibe proposes a change to its folder.</span> : null}
    </AskCard>
  );
}
