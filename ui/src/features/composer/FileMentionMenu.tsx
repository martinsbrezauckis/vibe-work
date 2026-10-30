/**
 * The @ menu above the message box: file names (name and muted folder), then the folder's other sessions (title and
 * the muted word Session). Depends on the mention rows, the vocabulary and the slash menu classes; called by Composer.
 */
import { useEffect, useRef, type ReactElement } from "react";
import { TEXT, vocabulary } from "../../vocabulary";
import { type MentionRow, mentionName } from "./model/mentions";

function rowText(row: MentionRow): { id: string; name: string; muted: string } {
  if (row.kind === "session")
    return {
      id: row.session.id,
      name: row.session.title,
      muted: vocabulary.workNames.session.name,
    };
  const { name, folder } = mentionName(row.hit.path);
  return { id: row.hit.path, name, muted: folder };
}

/**
 * Renders the existing menu styles and the approved searching, empty, folder and error rows. Session rows show as
 * soon as they match; file rows follow the core's answer.
 */
export function FileMentionMenu({
  rows,
  index,
  query,
  project,
  loading,
  error,
  onPick,
  onHover,
}: {
  rows: readonly MentionRow[];
  index: number;
  query: string;
  project: string | null;
  loading: boolean;
  error: string | null;
  onPick: (row: MentionRow) => void;
  onHover: (index: number) => void;
}): ReactElement {
  const selected = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const row = selected.current;
    const scroll: unknown = row === null ? undefined : Reflect.get(row, "scrollIntoView");
    if (row !== null && typeof scroll === "function") scroll.call(row, { block: "nearest" });
  }, [index, rows]);
  const note =
    project === null
      ? "Choose a folder first to mention its files."
      : rows.length > 0
        ? null
        : loading
          ? "Searching…"
          : `No file names match "${query}".`;
  return (
    <div
      data-testid="composer.file-mention-menu"
      className="slash"
      role="listbox"
      aria-label={TEXT.mentionMenu}
    >
      {error !== null ? (
        <div className="key-problem" role="alert">
          {error}
        </div>
      ) : note !== null ? (
        <div className="slash-empty" role="status">
          {note}
        </div>
      ) : (
        <div className="slash-list">
          {rows.map((row, i) => {
            const { id, name, muted } = rowText(row);
            return (
              <div
                key={`${row.kind}:${id}`}
                data-testid={
                  row.kind === "file"
                    ? "composer.file-mention-option"
                    : "composer.session-mention-option"
                }
                data-item-id={id}
                className="it"
                role="option"
                aria-selected={i === index}
                aria-label={`${name} ${muted}`.trim()}
                ref={i === index ? selected : undefined}
                onMouseDown={(event) => {
                  event.preventDefault();
                }}
                onClick={() => {
                  onPick(row);
                }}
                onMouseMove={() => {
                  if (i !== index) onHover(i);
                }}
              >
                <b>{name}</b>
                <span className="ui-help">{muted}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
