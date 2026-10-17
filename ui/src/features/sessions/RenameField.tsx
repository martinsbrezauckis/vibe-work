/**
 * Inline rename field for a session row or a project. Keyboard contract from ShellX `SessionTabs` @ e38c2b5c
 * (Enter commits, Escape cancels, blur commits, empty text changes nothing), with the field kept
 * from activating the row underneath it.
 * Depends on: react. Called by: SessionRow, ProjectGroupView, NewProjectField, projects/useProjectActions.
 * Invariants: commits at most once per edit session; an
 * unchanged or empty title is a cancel (no request is sent).
 */
import { useEffect, useRef } from "react";

/** The field; `initial` is the current title, `label` its accessible name. */
export function RenameField({
  initial,
  onCommit,
  onCancel,
  label = "Rename session",
  placeholder,
}: {
  initial: string;
  onCommit: (title: string) => void;
  onCancel: () => void;
  label?: string;
  placeholder?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const finish = (commit: boolean) => {
    if (done.current) return;
    done.current = true;
    const text = input.current?.value.trim() ?? "";
    if (commit && text !== "" && text !== initial) onCommit(text);
    else onCancel();
  };
  return (
    <input
      data-testid="sessions.rename-field-stop-propagation"
      ref={input}
      className="rename"
      aria-label={label}
      placeholder={placeholder}
      defaultValue={initial}
      maxLength={200}
      onClick={(e) => {
        e.stopPropagation();
      }}
      onBlur={() => {
        finish(true);
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          finish(true);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(false);
        }
      }}
    />
  );
}
