/**
 * One hunk of a changed file: header ("Hunk 2 · lines 12–18 · turn 3") with Keep / Revert and the block's ⋯,
 * and its lines (`HunkLines`: unified with the `DiffLineRow`, or side by side, with comment buttons). A right
 * click, the menu key or the ⋯ opens the block's menu (`changesMenus.ts`).
 * Depends on: model.ts, store.ts (actions), transcript rows (`DiffLineRow`), line comments, RowMenu.
 * Called by: FileReview.
 * Invariants: buttons only call the store; while the session is busy they are disabled (one action at a
 * time, so a second click never acts on a diff that is about to change).
 */
import { Button } from "../../components/Controls";
import { type ReactElement, useState } from "react";
import { createPortal } from "react-dom";
import { RowMenu, RowMenuButton, useRowMenu } from "../../components/RowMenu";
import { useApp } from "../shell/AppContext";
import { hunkMenu } from "./changesMenus";
import { useLineComments } from "./LineCommentsContext";

import { Icon } from "../../components/Icon";

import type { FileChange, Hunk } from "../../protocol/core";
import { HunkLines } from "./HunkLines";
import { hunkActions, hunkRange } from "./model";
import type { Changes } from "./store";

/** One hunk with its actions. */
export function HunkBlock(props: {
  changes: Changes;
  sessionId: string;
  file: FileChange;
  hunk: Hunk;
  index: number;
  layout: "split" | "unified";
  busy: boolean;
  /** No keep/revert buttons. */
  readOnly?: boolean;
}): ReactElement {
  const { changes, sessionId, file, hunk, index, layout, busy } = props;
  const label = `Hunk ${String(index + 1)} · ${hunkRange(hunk)}`;
  // (feel item 14): a kept hunk folds shut (200 ms) while the store applies it (a revert asks first); if it is still
  // here when the action ends (it failed, or the diff did not change), it opens again.
  const [leaving, setLeaving] = useState(false);
  const act = (run: () => Promise<unknown>) => {
    setLeaving(true);
    void run().finally(() => {
      setLeaving(false);
    });
  };
  const app = useApp();
  const comments = useLineComments();
  // The line under the pointer when the menu opened (for Add a line comment).
  const menu = useRowMenu<number | null>();
  const lineAt = (target: EventTarget) => {
    const index = (target instanceof Element ? target : null)
      ?.closest("[data-kind]")
      ?.querySelector<HTMLElement>("[data-line-index]")?.dataset["lineIndex"];
    return index === undefined ? null : Number(index);
  };
  const built =
    menu.at === null
      ? null
      : hunkMenu(
          app,
          changes,
          comments,
          sessionId,
          file,
          hunk,
          menu.at.target,
          act,
          props.readOnly === true,
        );
  return (
    <div
      data-testid="changes.hunk"
      data-item-id={props.hunk.id}
      className={`hunk${leaving ? " leaving" : ""}`}
      data-hunk-id={hunk.id}
      role="region"
      aria-label={label}
      onContextMenu={(event) => {
        menu.triggers(lineAt(event.target)).onContextMenu(event);
      }}
      onKeyDown={(event) => {
        menu.triggers(lineAt(event.target)).onKeyDown(event);
      }}
    >
      <div className="hunk-h">
        <span>{label}</span>
        <span className="sp" />
        <RowMenuButton
          testId="changes.hunk-more"
          itemId={hunk.id}
          label={`Actions for ${label}`}
          expanded={menu.at !== null}
          onOpen={(button) => {
            menu.openBelow(button, null);
          }}
        />
        {hunkActions(file) && props.readOnly !== true && (
          <span className="hunk-acts">
            <Button
              data-testid="changes.hunk-block-act"
              type="button"
              appearance="compact"
              disabled={busy}
              data-action="keep-hunk"
              aria-label={`Keep ${label}`}
              onClick={() => {
                act(() => changes.keepHunk(sessionId, file, hunk));
              }}
            >
              <Icon name="check" size={14} />
              Keep
            </Button>
            <Button
              data-testid="changes.hunk-block-act-2"
              type="button"
              shape="outlined"
              variant="danger"
              disabled={busy}
              data-action="revert-hunk"
              aria-label={`Revert ${label}`}
              onClick={() => void changes.revertHunk(sessionId, file, hunk)}
            >
              <Icon name="undo-2" size={14} />
              Revert
            </Button>
          </span>
        )}
      </div>
      <HunkLines sessionId={sessionId} path={file.path} hunk={hunk} layout={layout} />
      {built !== null &&
        menu.at !== null &&
        createPortal(
          <RowMenu
            testId="changes.hunk-menu"
            at={menu.at}
            label={built.label}
            heading={built.heading}
            items={built.items}
            onClose={menu.close}
          />,
          document.body,
        )}
    </div>
  );
}
