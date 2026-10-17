/**
 * The lines of one hunk: unified (the `DiffLineRow`) or side by side,
 * each row with a "+" comment button in the gutter (always in the DOM and in tab order, shown on hover and
 * focus), a count marker on rows that carry comments, the comments inline under their last row (unified) or
 * under the hunk (side by side), and the form under the rows being commented. Shift+click (or Shift+Enter on the
 * button) extends the draft to a range inside the hunk.
 * Depends on: model.ts, lineCommentModel, LineCommentsContext, LineCommentForm/Thread, transcript `DiffLineRow`.
 * Called by: HunkBlock.
 * Invariants: without a comments controller the rows render exactly as before (no buttons); comments never
 * change what the hunk shows.
 */
import { Fragment, type ReactElement } from "react";

import type { Hunk, LineCommentView } from "../../protocol/core";
import { useStore } from "../../state/createStore";
import { DiffLineRow } from "../transcript/rows/DiffView";
import { LineCommentForm } from "./LineCommentForm";
import { LineCommentThread } from "./LineCommentThread";
import { useLineComments } from "./LineCommentsContext";
import { commentsAt, threadsEndingAt } from "./lineCommentModel";
import type { LineComments } from "./lineComments";
import { splitRows, toDiffLine } from "./model";

const NONE: LineCommentView[] = [];

/** Where a hunk is: its session, file and layout. */
export interface HunkPlace {
  sessionId: string;
  path: string;
  hunk: Hunk;
  layout: "split" | "unified";
}

/** The hunk's lines. */
export function HunkLines(place: HunkPlace): ReactElement {
  const ctl = useLineComments();
  return ctl === null ? <Plain {...place} /> : <Commentable {...place} ctl={ctl} />;
}

/** Rows without comments. */
function Plain({ hunk, layout }: HunkPlace): ReactElement {
  if (layout === "split") return <SplitCells hunk={hunk} />;
  return (
    <div className="diff">
      {hunk.lines.map((line, i) => (
        <DiffLineRow key={i} line={toDiffLine(line)} />
      ))}
    </div>
  );
}

/** The two columns of the side-by-side view; `gutter` renders the comment button of one line index. */
function SplitCells({
  hunk,
  gutter,
}: {
  hunk: Hunk;
  gutter?: (index: number) => ReactElement;
}): ReactElement {
  const lines = hunk.lines.map(toDiffLine);
  const index = new Map(lines.map((l, i) => [l, i]));
  const rows = splitRows(lines);
  const cell = (side: "l" | "r", line: (typeof rows)[number]["left"], i: number) => {
    if (line === null) return <div key={`${side}${String(i)}`} className="sl e" />;
    const cls = line.kind === "add" ? "sl a" : line.kind === "del" ? "sl d" : "sl";
    const no = side === "l" ? line.oldNo : line.newNo;
    const at = index.get(line);
    // Context lines appear on both sides: the button lives on the right ("After") one only.
    const button = at !== undefined && (side === "r" || line.kind === "del") ? gutter?.(at) : null;
    return (
      <div key={`${side}${String(i)}`} className={cls} data-kind={line.kind}>
        {button}
        <span className="n">{no ?? ""}</span>
        <span>{line.text}</span>
      </div>
    );
  };
  return (
    <div className="split">
      <div aria-label="Before">{rows.map((r, i) => cell("l", r.left, i))}</div>
      <div aria-label="After">{rows.map((r, i) => cell("r", r.right, i))}</div>
    </div>
  );
}

function Commentable({
  sessionId,
  path,
  hunk,
  layout,
  ctl,
}: HunkPlace & { ctl: LineComments }): ReactElement {
  const all = useStore(ctl.store, (s) => s.comments[sessionId] ?? NONE);
  const draft = useStore(ctl.store, (s) =>
    s.draft !== null && s.draft.hunkId === hunk.id && s.draft.path === path ? s.draft : null,
  );
  const total = hunk.lines.length;
  const send = (id: string) => {
    void ctl.send(sessionId, [id]);
  };
  const remove = (id: string) => {
    void ctl.remove(sessionId, id);
  };
  const button = (i: number) => {
    const here = commentsAt(all, path, hunk.id, i);
    const line = hunk.lines[i];
    const where = line?.newNo ?? line?.oldNo;
    return (
      <>
        <button
          data-testid="changes.hunk-lines-begin"
          type="button"
          className="lc-add"
          data-line-index={i}
          aria-label={`Comment on ${path}, ${line?.newNo === null ? "removed " : ""}line ${String(where ?? "")}`}
          onClick={(event) => {
            ctl.begin(sessionId, path, hunk.id, i, total, event.shiftKey);
          }}
        />
        {here.length > 0 && (
          <span
            className="lc-mark"
            role="img"
            data-count={here.length}
            aria-label={`${String(here.length)} comment${here.length === 1 ? "" : "s"} on this line`}
          />
        )}
      </>
    );
  };
  const form =
    draft === null ? null : (
      <LineCommentForm
        path={path}
        lines={hunk.lines.slice(draft.from, draft.to + 1)}
        save={(text, andSend) => {
          void ctl.save(text, andSend);
        }}
        cancel={() => {
          ctl.cancel();
        }}
      />
    );
  if (layout === "split") {
    const mine = all.filter(
      (c): c is LineCommentView & { state: "attached" } =>
        c.state === "attached" && c.comment.path === path && c.hunkId === hunk.id,
    );
    return (
      <>
        <SplitCells hunk={hunk} gutter={button} />
        {mine.map((c) => (
          <LineCommentThread key={c.comment.id} view={c} send={send} remove={remove} />
        ))}
        {form}
      </>
    );
  }
  return (
    <div className="diff">
      {hunk.lines.map((line, i) => {
        const selected = draft !== null && i >= draft.from && i <= draft.to;
        return (
          <Fragment key={i}>
            <div
              className="lc-row"
              data-selected={selected}
              data-commented={commentsAt(all, path, hunk.id, i).length > 0}
            >
              {button(i)}
              <DiffLineRow line={toDiffLine(line)} />
            </div>
            {threadsEndingAt(all, path, hunk.id, i).map((c) => (
              <LineCommentThread key={c.comment.id} view={c} send={send} remove={remove} />
            ))}
            {draft !== null && draft.to === i ? form : null}
          </Fragment>
        );
      })}
    </div>
  );
}
