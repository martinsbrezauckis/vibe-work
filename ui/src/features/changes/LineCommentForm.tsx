/**
 * The form under the rows being commented: a text box, "Comment", "Comment and send to Vibe" and
 * "Cancel". Keyboard: Ctrl/Cmd+Enter saves, Escape cancels; the box takes focus when the form opens.
 * Depends on: lineComments.ts (Draft), lineCommentModel (range text). Called by: HunkLines.
 * Invariants: renders only; the comment is plain text; it says where it is kept (Vibe Work's data, not the repo).
 */
import { Button } from "../../components/Controls";
import { formatKeys } from "../../state/shortcuts";
import { type ReactElement, useEffect, useRef, useState } from "react";

import type { DiffLine } from "../../protocol/core";
import { rangeText } from "./lineCommentModel";

/** The form. `lines` are the rows being commented. */
export function LineCommentForm(props: {
  path: string;
  lines: readonly DiffLine[];
  save: (text: string, andSend: boolean) => void;
  cancel: () => void;
}): ReactElement {
  const { path, lines, save, cancel } = props;
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    box.current?.focus();
  }, []);
  const empty = text.trim() === "";
  return (
    <form
      className="lc-form"
      aria-label={`Comment on ${path}, ${rangeText(lines)}`}
      onSubmit={(event) => {
        event.preventDefault();
      }}
    >
      <label htmlFor="lc-text">
        Comment on {path}, {rangeText(lines)}
      </label>
      <textarea
        data-testid="changes.line-comment-form-set-text"
        id="lc-text"
        ref={box}
        value={text}
        placeholder="Vibe gets it with the file and the lines."
        onChange={(event) => {
          setText(event.target.value);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            cancel();
          } else if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && !empty) {
            event.preventDefault();
            save(text, false);
          }
        }}
      />
      <div className="lc-form-row">
        <Button
          data-testid="changes.line-comment-form-save"
          type="button"
          appearance="compact"
          disabled={empty}
          onClick={() => {
            save(text, false);
          }}
        >
          Comment
        </Button>
        <Button
          data-testid="changes.line-comment-form-save-2"
          type="button"
          appearance="compact"
          disabled={empty}
          data-action="comment-send"
          onClick={() => {
            save(text, true);
          }}
        >
          Comment and send to Vibe
        </Button>
        <Button
          data-testid="changes.line-comment-form-mini"
          type="button"
          appearance="compact"
          onClick={cancel}
        >
          Cancel
        </Button>
        <span className="lc-note">
          Kept in Vibe Work, not in the repo. {formatKeys("Mod+Enter")} comments.
        </span>
      </div>
    </form>
  );
}
