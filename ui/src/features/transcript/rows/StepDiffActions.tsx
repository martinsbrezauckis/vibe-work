/** Finished edit header links to the existing review and file views. Called only by StepBody. */
import { Button } from "../../../components/Controls";
import type { ReactElement } from "react";
import { useApp } from "../../shell/AppContext";
import { useFileSessionId } from "../../files/FileRef";
import { firstChangedLine, type FileDiff } from "../model/diff";

/** Navigation for an applied edit with a path; shared diff previews do not supply these actions. */
export function StepDiffActions({
  diff,
  sessionId,
}: {
  diff: FileDiff;
  sessionId: string | null;
}): ReactElement {
  const { changes, git, files, shell, toasts } = useApp();
  const transcriptSession = useFileSessionId();
  const source = sessionId ?? transcriptSession;
  const name = diff.path.split(/[\\/]/).pop() ?? diff.path;
  const line = firstChangedLine(diff);
  return (
    <>
      <Button
        data-testid="transcript.step-diff-review"
        type="button"
        appearance="compact"
        aria-label={`Review changes to ${name}`}
        title="Review this change in the Changes tab (keep or undo it)"
        onClick={() => {
          if (changes === null || source === null) return;
          const open = async () => {
            const owner = changes.owner(source);
            shell.actions.selectSession(owner);
            const repo = git?.store.get().status[owner]?.repo;
            if (git !== null && repo != null && repo !== ".") await git.selectRepo(owner, ".");
            void changes.review(owner, diff.path, line);
            shell.actions.openTab("changes");
          };
          void open();
        }}
      >
        Review
      </Button>
      <Button
        data-testid="transcript.step-diff-open-file"
        type="button"
        appearance="compact"
        aria-label={`Open ${name} at line ${String(line)}`}
        title={`Open ${name} at the changed line`}
        onClick={() => {
          if (files === null || source === null) return;
          shell.actions.selectSession(source);
          shell.actions.openTab("file");
          void files.actions.openAt(source, diff.path, line).catch((error: unknown) => {
            toasts.push({
              kind: "warn",
              bellKind: "failure",
              text: error instanceof Error ? error.message : "Could not open the file.",
            });
          });
        }}
      >
        Open file
      </Button>
    </>
  );
}
