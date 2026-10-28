/** A saved snapshot can remain open after its editable partner closes. Uses the existing read-only code editor and file workspace. */
import { Button } from "../../components/Controls";
import type { Files, Shown } from "../../state/files";
import { savedWords } from "../files/openFiles";
import { CodeEditor } from "./CodeEditor";
import { useRef } from "react";
import { registerCodeActions } from "../files/liveEditors";
export function SavedDocument({
  files,
  sessionId,
  file,
  shown,
  text,
  savedName = "saved",
  savedLabel,
}: {
  files: Files;
  sessionId: string;
  file: string;
  shown: Shown;
  text: string;
  savedName?: "saved" | "disk" | "commit";
  savedLabel?: string;
}) {
  const unregister = useRef<(() => void) | null>(null);
  const words = savedWords(
    savedLabel === undefined
      ? { shown, used: 0, changed: false, savedName }
      : { shown, used: 0, changed: false, savedName, savedLabel },
  );
  return (
    <div className="file-tab">
      <div className="file-head">
        <span className="file-path">
          {shown.display} · {words.tab} · read-only
        </span>
        <Button
          data-testid="files.close-saved-document"
          appearance="compact"
          onClick={() => {
            files.workspace.close(sessionId, file);
          }}
        >
          Close saved version
        </Button>
      </div>
      <div className="file-body">
        <CodeEditor
          onReady={(handle) => {
            unregister.current?.();
            unregister.current =
              handle === null ? null : registerCodeActions(sessionId, file, handle);
          }}
          value={text}
          onChange={() => undefined}
          readOnly
          label={
            savedName === "saved"
              ? "last saved version, read-only"
              : savedName === "commit"
                ? words.line
                : "Disk version, read-only"
          }
          filename={shown.display}
          line={shown.line}
        />
      </div>
    </div>
  );
}
