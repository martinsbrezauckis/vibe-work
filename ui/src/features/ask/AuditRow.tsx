/**
 * The one-line audit row a closed ask leaves in the transcript: the outcome square and its
 * exact words from the core's record, who answered from what this window sent, and "and turned on “{control}”"
 * for an "Always" answer. Depends on: model.ts (`auditLine`), the core and transcript stores. Called by: the ask
 * adapters (a card that closed) and the steps group (answered callbacks).
 */
import type { ReactElement } from "react";
import type { AskRecord } from "../../protocol/core";
import { useTranscriptUi } from "../transcript/hooks";
import { type AuditLine, auditLine } from "./model";

/** Renders `line` (already worded). */
export function AuditText({
  line,
  callbackId,
  entryId,
}: {
  line: AuditLine;
  callbackId?: string;
  entryId?: string;
}): ReactElement {
  return (
    <div
      className="audit"
      role="note"
      {...(callbackId === undefined ? {} : { "data-callback-id": callbackId })}
      {...(entryId === undefined ? {} : { "data-entry-id": entryId })}
      {...(line.title === undefined ? {} : { title: line.title })}
    >
      <span className={`px ${line.square}`} aria-hidden="true" />
      {line.text}
    </div>
  );
}

/** The audit row of the record of callback `callbackId`. */
export function AuditRow({
  record,
  summary,
  callbackId,
  entryId,
}: {
  record: AskRecord;
  summary: string;
  callbackId: string;
  entryId?: string;
}): ReactElement | null {
  const via = useTranscriptUi((s) => s.answered[callbackId]);
  const always = useTranscriptUi((s) => s.always[callbackId]);
  const line = auditLine(record, summary, via, always);
  return line === null ? null : (
    <AuditText
      line={line}
      callbackId={callbackId}
      {...(entryId === undefined ? {} : { entryId })}
    />
  );
}
