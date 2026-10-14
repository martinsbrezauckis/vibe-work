/**
 * One transcript row: picks the component for the row's kind. Memoised on the row object, which the
 * model keeps identical while nothing in it changed (`model/memo.ts`), so a streamed patch re-renders
 * only its own row.
 * Depends on: model/build, the row components. Called by: Transcript.
 */
import { type ReactElement, memo } from "react";

import type { Row } from "../model/build";
import { detailOf, entryId } from "../model/entries";
import { ApprovalCard } from "./ApprovalCard";
import { AssistantRow, UserRow } from "./MessageRows";
import { QuestionCard } from "./QuestionCard";
import { DividerRow, NoticeRow, OtherRow, TurnEndRow } from "./SmallRows";
import { StepsGroup } from "./StepsGroup";

function Body({ row, sessionId }: { row: Row; sessionId: string }): ReactElement {
  switch (row.kind) {
    case "user":
      return <UserRow entry={row.entry} steer={row.steer} sessionId={sessionId} />;
    case "assistant":
      return <AssistantRow entry={row.entry} searches={row.searches} rowKey={row.key} />;
    case "steps":
      return <StepsGroup row={row} />;
    case "callback":
      // a subagent's card answers for the child session (`answerFor`), not the transcript's own.
      return detailOf(row.entry)["kind"] === "user_input" ? (
        <QuestionCard entry={row.entry} sessionId={row.answerFor ?? sessionId} />
      ) : detailOf(row.entry)["kind"] === "approval" ? (
        <ApprovalCard entry={row.entry} sessionId={row.answerFor ?? sessionId} />
      ) : (
        <OtherRow entry={row.entry} />
      );
    case "notice":
      return <NoticeRow entry={row.entry} />;
    case "hook":
      return (
        <div className="other-row" data-entry-id={entryId(row.entry)} data-kind="hook-outcome">
          {row.text}
        </div>
      );
    case "divider":
      return <DividerRow entry={row.entry} />;
    case "turnEnd":
      return <TurnEndRow turn={row.turn} />;
    case "other":
      return <OtherRow entry={row.entry} />;
  }
}

/** Renders one row inside the transcript's centred column. */
export const RowView = memo(function RowView({
  row,
  sessionId,
}: {
  row: Row;
  sessionId: string;
}): ReactElement {
  return (
    <div className="tx-row" data-row-kind={row.kind} data-row-key={row.key}>
      <Body row={row} sessionId={sessionId} />
    </div>
  );
});
