/**
 * The "review your changes" view of an instruction editor: every line of the new text, with removed
 * lines struck and added lines marked, so the user sees exactly what will be written.
 * Depends on: state/lineDiff. Called by: DocEditor.
 * Invariants: the marks are text as well as colour (a leading "+" / "-" the screen reader also gets as
 * words); unchanged lines far from a change are folded so a long file stays readable.
 */
import { type DiffLine, diffCounts } from "../../state/lineDiff";

/** Unchanged lines kept around a change before the rest is folded. */
const CONTEXT = 3;

interface Shown {
  line: DiffLine | null;
  folded: number;
}

/** Keeps changed lines and `CONTEXT` lines around them; runs of other lines become one fold. */
function fold(lines: readonly DiffLine[]): Shown[] {
  const keep = lines.map((l) => l.kind !== "same");
  const near = keep.map((_, i) => {
    for (let d = -CONTEXT; d <= CONTEXT; d += 1) if (keep[i + d] === true) return true;
    return false;
  });
  const out: Shown[] = [];
  let hidden = 0;
  lines.forEach((line, i) => {
    if (near[i] === true) {
      if (hidden > 0) out.push({ line: null, folded: hidden });
      hidden = 0;
      out.push({ line, folded: 0 });
    } else {
      hidden += 1;
    }
  });
  if (hidden > 0) out.push({ line: null, folded: hidden });
  return out;
}

const MARK: Record<DiffLine["kind"], string> = { same: " ", add: "+", del: "-" };
const WORDS: Record<DiffLine["kind"], string> = { same: "", add: "Added: ", del: "Removed: " };

/** The diff block. */
export function DiffPreview({ lines }: { lines: readonly DiffLine[] }) {
  const { added, removed, finalNewlineChanged } = diffCounts(lines);
  return (
    <div className="diff-preview" data-testid="instructions-diff">
      <p className="key-line">
        {added} {added === 1 ? "line" : "lines"} added, {removed} {removed === 1 ? "line" : "lines"}{" "}
        removed.
        {finalNewlineChanged ? " The line break at the end of the file changes." : ""}
      </p>
      <pre className="diff-lines" aria-label="Changes to save">
        {fold(lines).map((shown, i) =>
          shown.line === null ? (
            <span key={i} className="diff-fold">
              {`… ${String(shown.folded)} unchanged ${shown.folded === 1 ? "line" : "lines"}\n`}
            </span>
          ) : (
            <span
              key={i}
              className={`diff-line diff-${shown.line.kind}`}
              data-kind={shown.line.kind}
            >
              <span className="sr">{WORDS[shown.line.kind]}</span>
              {`${MARK[shown.line.kind]} ${shown.line.text}\n`}
            </span>
          ),
        )}
      </pre>
    </div>
  );
}
