/**
 * "Not in a project": sessions the user took out of every project (or whose project was deleted), and
 * sessions without a folder. A drop target: a session dropped here leaves its project. Collapsible like a project.
 * Depends on: AppContext, hooks, listDrop (the column decides drops; the section renders the mark), SessionRow,
 * listModel, components/Icon. Called by: SessionList.
 * Invariants: shown whenever the groups are loaded and it holds a session; an empty one stays hidden ("Not in a project 0" is noise) except while a session is being dragged, when it appears as a drop target. Fold state and
 * row identities survive filtering.
 */

import { Icon } from "../../components/Icon";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { UNFILED_KEY, useDropMark, useSessionDragging } from "./listDrop";
import type { RowNode } from "./listModel";
import { SessionRows } from "./SessionRows";

function count(nodes: readonly RowNode[]): number {
  return nodes.reduce((n, node) => n + 1 + count(node.children), 0);
}

/** The section. */
export function UnfiledSection({
  rows,
  shown = rows,
}: {
  rows: readonly RowNode[];
  shown?: readonly RowNode[];
}) {
  const { shell } = useApp();
  const collapsed = useShellState((s) => s.collapsed[UNFILED_KEY] === true);
  const drop = useDropMark(UNFILED_KEY);
  const dragging = useSessionDragging();
  const open = !collapsed;
  return (
    <div
      className="proj unfiled"
      hidden={shown.length === 0 && !dragging}
      data-open={open}
      data-project={UNFILED_KEY}
      data-drop-key={UNFILED_KEY}
      data-drop-id=""
      data-drop={drop}
    >
      <div className="proj-bar">
        <button
          data-testid="sessions.unfiled-section-toggle-project"
          type="button"
          className="proj-h"
          aria-expanded={open}
          title="Sessions in no project. Drop a session here to take it out of its project."
          onClick={() => {
            shell.actions.toggleProject(UNFILED_KEY);
          }}
        >
          <span className="caret" aria-hidden="true">
            <Icon name="chevron-right" size={12} />
          </span>
          <span className="fold" aria-hidden="true">
            <Icon name="message-square" size={16} />
          </span>
          <span className="pname">Not in a project</span>
          <span className="cnt">{count(shown)}</span>
        </button>
      </div>
      <div className="proj-sessions" hidden={!open}>
        <SessionRows rows={rows} shown={shown} />
      </div>
    </div>
  );
}
