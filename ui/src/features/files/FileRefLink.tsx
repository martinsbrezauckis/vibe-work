/** Resolved transcript file link with the shared file menu, or the ambiguous chooser; RowMenu owns keyboard and focus. */
import { type ReactNode, useState } from "react";
import { createPortal } from "react-dom";
import { RowMenu, type MenuItem } from "../../components/RowMenu";
import type { RefState, Shown } from "../../state/files";
import { useApp } from "../shell/AppContext";
import { editableKind, runFileRefAction, type RefAction } from "./fileRefActions";
import { type FileTarget, fileHeading, fileMenuItems } from "./fileMenu";

/** A linked file name changes no text and never chooses between ambiguous files on its own. */
export function FileRefLink({
  sessionId,
  path,
  line,
  state,
  children,
}: {
  sessionId: string;
  path: string;
  line: number | null;
  state: Extract<RefState, { status: "ok" | "choices" }>;
  children: ReactNode;
}) {
  const app = useApp();
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const choose = state.status === "choices";
  const linkTarget = (): FileTarget => {
    const cwd = app.core?.store.get().sessions[sessionId]?.session["cwd"];
    const ok = state.status === "ok" ? state : null;
    return {
      owner: sessionId,
      folder: typeof cwd === "string" ? cwd : "",
      path: ok?.display ?? path,
      line,
      kind: ok?.kind ?? null,
      shown: ok === null ? null : { file: ok.file, display: ok.display, line },
    };
  };
  const heading = choose ? `${String(state.total)} files named ${path}` : fileHeading(linkTarget());
  const hint = choose
    ? `${heading}: click to choose`
    : `Open ${state.display}${line === null ? "" : ` at line ${String(line)}`} in the File tab`;
  const run = (target: Omit<Shown, "line">, item: RefAction) => {
    void runFileRefAction(app, sessionId, { ...target, line }, item).catch(() => undefined);
  };
  const items: MenuItem[] = choose
    ? state.choices.map((choice) => {
        const slash = choice.display.lastIndexOf("/");
        return {
          id: choice.file,
          name: choice.display,
          label: (
            <>
              <span className="file-choice-folder">{choice.display.slice(0, slash + 1)}</span>
              {choice.display.slice(slash + 1)}
            </>
          ),
          run: () => {
            run(choice, "open");
          },
          secondary: {
            label: `Edit ${choice.display}`,
            disabled: !editableKind(choice.kind),
            run: () => {
              run(choice, "edit");
            },
          },
        };
      })
    : fileMenuItems(app, linkTarget(), "chat");
  const openMenu = (button: HTMLButtonElement, x?: number, y?: number) => {
    button.focus();
    const rect = button.getBoundingClientRect();
    setAt(x === undefined || y === undefined ? { x: rect.left, y: rect.bottom } : { x, y });
  };
  return (
    <>
      <button
        data-testid="files.file-ref-open"
        type="button"
        className="fref"
        title={hint}
        aria-label={hint}
        aria-haspopup="menu"
        aria-expanded={at !== null}
        data-file-ref={path}
        data-file-state={state.status}
        onClick={(event) => {
          if (choose) openMenu(event.currentTarget);
          else run(state, "open");
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          openMenu(event.currentTarget, event.clientX, event.clientY);
        }}
        onKeyDown={(event) => {
          if (event.key !== "ContextMenu" && !(event.key === "F10" && event.shiftKey)) return;
          event.preventDefault();
          event.stopPropagation();
          openMenu(event.currentTarget);
        }}
      >
        {children}
      </button>
      {at !== null &&
        createPortal(
          <RowMenu
            testId={choose ? "files.ref-chooser" : "files.ref-menu"}
            at={at}
            label={choose ? heading : `Actions for ${path}`}
            heading={heading}
            footer={
              choose && state.total > state.choices.length
                ? `Showing ${String(state.choices.length)} of ${String(state.total)}. Name the file with its folder to narrow it.`
                : undefined
            }
            items={items}
            onClose={() => {
              setAt(null);
            }}
          />,
          document.body,
        )}
    </>
  );
}
