/**
 * The notice above an open editor after its file changed on disk (reloaded, merged or both changed the same
 * lines), with its choices. Depends on: Controls, model/diskNotice. Called by: FileEditor.
 * Invariants: the sentence states what already happened; every button changes only the editor's draft (Save still
 * writes), except Compare side by side, which opens the disk version beside the editor.
 */
import type { ReactElement } from "react";
import { Button } from "../../components/Controls";
import type { FileEdit } from "../../state/fileEdits";
import { applyChoice, noticeText, takeLabel, type DiskChoice } from "./model/diskNotice";

export function DiskNotice({
  draft,
  name,
  byVibe,
  onChoose,
  onShowChanges,
  onCompare,
}: {
  draft: FileEdit;
  name: string;
  byVibe: boolean;
  onChoose: (change: (draft: FileEdit) => FileEdit) => void;
  onShowChanges: (line: number) => void;
  onCompare: () => void;
}): ReactElement | null {
  const notice = draft.notice;
  if (notice === null) return null;
  const choose = (choice: DiskChoice) => {
    onChoose((d) => applyChoice(d, choice));
  };
  const first = notice.lines[0];
  return (
    <div
      className="server-note file-edit-notice"
      role={notice.kind === "overlap" ? "alert" : "status"}
      data-notice={notice.kind}
    >
      <span>{noticeText(notice, name, byVibe)}</span>
      {notice.kind === "reloaded" && first !== undefined && (
        <Button
          data-testid="files.disk-notice-show-changes"
          type="button"
          appearance="compact"
          onClick={() => {
            onShowChanges(first);
          }}
        >
          Show changes
        </Button>
      )}
      {notice.kind === "merged" && (
        <Button
          data-testid="files.disk-notice-undo-merge"
          type="button"
          appearance="compact"
          disabled={draft.saving}
          onClick={() => {
            choose("undoMerge");
          }}
        >
          Undo merge
        </Button>
      )}
      {notice.kind === "overlap" && (
        <>
          <Button
            data-testid="files.disk-notice-keep-yours"
            type="button"
            appearance="compact"
            disabled={draft.saving}
            onClick={() => {
              choose("keepYours");
            }}
          >
            Keep yours
          </Button>
          <Button
            data-testid="files.disk-notice-take-vibes"
            type="button"
            appearance="compact"
            disabled={draft.saving}
            onClick={() => {
              choose("takeVibes");
            }}
          >
            {takeLabel(byVibe)}
          </Button>
          <Button
            data-testid="files.disk-notice-compare"
            type="button"
            appearance="compact"
            onClick={onCompare}
          >
            Compare side by side
          </Button>
        </>
      )}
      {notice.kind !== "overlap" && (
        <Button
          data-testid="files.disk-notice-got-it"
          type="button"
          appearance="compact"
          onClick={() => {
            choose("gotIt");
          }}
        >
          Got it
        </Button>
      )}
    </div>
  );
}
