/** File names above the message box. Depends on core hits and slash menu classes; called by Composer. */
import { useEffect, useRef, type ReactElement } from "react";
import type { FileHit } from "../../protocol/core";
import { mentionName } from "./model/mentions";

/** Renders the existing menu styles and the approved searching, empty, folder and error rows. */
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
  rows: readonly FileHit[];
  index: number;
  query: string;
  project: string | null;
  loading: boolean;
  error: string | null;
  onPick: (hit: FileHit) => void;
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
      : loading
        ? "Searching…"
        : rows.length === 0
          ? `No file names match "${query}".`
          : null;
  return (
    <div
      data-testid="composer.file-mention-menu"
      className="slash"
      role="listbox"
      aria-label="Files"
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
          {rows.map((hit, i) => {
            const { name, folder } = mentionName(hit.path);
            return (
              <div
                key={hit.path}
                data-testid="composer.file-mention-option"
                data-item-id={hit.path}
                className="it"
                role="option"
                aria-selected={i === index}
                aria-label={`${name} ${folder}`.trim()}
                ref={i === index ? selected : undefined}
                onMouseDown={(event) => {
                  event.preventDefault();
                }}
                onClick={() => {
                  onPick(hit);
                }}
                onMouseMove={() => {
                  if (i !== index) onHover(i);
                }}
              >
                <b>{name}</b>
                <span className="ui-help">{folder}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
