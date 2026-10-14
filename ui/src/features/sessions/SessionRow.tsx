/**
 * One session in the list: badge, title, unread dot, voice chat state, "local" and "Last in CLI" tags, the folder chip (for a session outside its project's main folder), a "more" button and child fold controls.
 * Right click, the context-menu key, Shift+F10 and the more button open the same action menu; F2 renames,
 * Delete asks for the native delete confirmation. The row can be dragged onto a project or "Not in a
 * project" (with its children), and "Move to project…" in the menu is the keyboard way to do the same.
 * Depends on: AppContext, hooks, useGroups, dnd, Badge, RenameField, RowMenu, listModel (`RowNode`),
 * components/Icon. Called by: SessionRows. Invariants: the row never changes data
 * itself, it calls `sessions.*` / `groups.*` actions; the main button carries `data-session-id` and
 * `aria-current`; SessionRows owns child visibility and stable sibling identities.
 */
import { useState } from "react";

import { Icon } from "../../components/Icon";
import { createStore, useStore } from "../../state/createStore";
import { BADGE_LABEL } from "../../state/sessionSummary";
import { NO_SESSION_TOOLS } from "../../state/sessionTools";
import { useApp } from "../shell/AppContext";
import { useCoreState, useShellState } from "../shell/hooks";
import { VoiceChatTag } from "../voice/VoiceChatTag";
import { Badge } from "./Badge";
import { dragSessions } from "./dnd";
import { beginSessionDrag } from "./listDrop";
import type { RowNode } from "./listModel";
import { RenameField } from "./RenameField";
import { type MenuItem, RowMenu } from "./RowMenu";
import { useGroupActions, useGroups } from "./useGroups";

/** Stands in for the retry store when the app has no core (no session actions). */
const NO_RETRIES = createStore<ReadonlySet<string>>(new Set());

/** The row's session and its nested children: they move together. */
function ids(node: RowNode): string[] {
  return [node.session.id, ...node.children.flatMap(ids)];
}

/** One row; SessionRows renders descendants as stable siblings. */
export function SessionRow({
  node,
  depth = 0,
  hidden = false,
}: {
  node: RowNode;
  depth?: number;
  hidden?: boolean;
}) {
  const app = useApp();
  const { shell, sessions } = app;
  const s = node.session;
  const selected = useShellState((x) => x.selected === s.id);
  const expanded = useShellState((x) => x.expanded[s.id] === true);
  const cliId = useCoreState((x) => x.index.continueSessionId);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const hasChildren = node.children.length > 0;
  const retrying = useStore(sessions?.retrying ?? NO_RETRIES, (x) => x.has(s.id));
  const temporary = useStore(app.sessionTools?.store ?? NO_SESSION_TOOLS, (x) =>
    x.temporary.includes(s.id),
  );
  const groups = useGroupActions();
  const grouped = useGroups((g) => g.data !== null);
  const [dragging, setDragging] = useState(false);

  // A stuck session offers the two ways out first.
  const stuckItems: MenuItem[] = !s.stuck
    ? []
    : [
        retrying
          ? { id: "stop", label: "Stop trying", run: () => void sessions?.cancelReopen(s.id) }
          : { id: "retry", label: "Try again", run: () => void sessions?.retryReopen(s.id) },
        {
          id: "continue",
          label: "Continue in a new session",
          run: () => void sessions?.continueInNew(s.id),
          separated: true,
        },
      ];
  const items: MenuItem[] = [
    ...stuckItems,
    { id: "open", label: "Open", run: () => void sessions?.open(s.id), separated: s.stuck },
    {
      id: "rename",
      label: "Rename",
      run: () => {
        setRenaming(true);
      },
    },
    {
      id: "pin",
      label: s.pinned ? "Unpin" : "Pin to the top",
      run: () => void sessions?.pin(s.id, !s.pinned),
      disabled: s.archived,
    },
    {
      id: "archive",
      label: s.archived ? "Unarchive" : "Archive",
      run: () => void sessions?.archive(s.id, !s.archived),
    },
    ...(groups !== null && grouped
      ? [
          {
            id: "move",
            label: "Move to project...",
            run: () => {
              groups.openMover(ids(node), menu ?? { x: 24, y: 24 });
            },
          },
        ]
      : []),
    {
      // Export the whole session as Markdown, through the native save dialog.
      id: "export",
      label: "Export as Markdown...",
      run: () => void app.sessionTools?.exportSession(s.id),
    },
    {
      id: "reveal",
      label: "Show folder",
      run: () => void sessions?.revealFolder(s.id),
      disabled: s.cwd === null,
    },
    {
      id: "delete",
      label: "Delete...",
      run: () => void sessions?.remove(s.id),
      danger: true,
      separated: true,
    },
  ];
  const openMenuAt = (x: number, y: number) => {
    setMenu({ x, y });
  };
  const status = BADGE_LABEL[s.badge];

  return (
    <div className="srow-wrap" hidden={hidden}>
      <div
        className={`srow${s.unread ? " is-unread" : ""}${s.archived ? " is-archived" : ""}${dragging ? " is-dragging" : ""}`}
        data-depth={depth}
        style={depth > 0 ? { paddingLeft: depth * 14 } : undefined}
        draggable={groups !== null && grouped && !renaming}
        onDragStart={(e) => {
          e.stopPropagation();
          dragSessions(e.dataTransfer, ids(node));
          beginSessionDrag();
          setDragging(true);
        }}
        onDragEnd={() => {
          setDragging(false);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          openMenuAt(e.clientX, e.clientY);
        }}
      >
        {hasChildren ? (
          <button
            type="button"
            className="twist"
            aria-expanded={expanded}
            aria-label={`${expanded ? "Hide" : "Show"} ${String(node.children.length)} child sessions`}
            title={`${expanded ? "Hide" : "Show"} ${String(node.children.length)} child sessions`}
            onClick={() => {
              shell.actions.toggleExpanded(s.id);
            }}
          >
            <Icon name="chevron-right" size={12} />
          </button>
        ) : (
          <span className="twist-gap" />
        )}
        {renaming ? (
          <div className="srow-main">
            <Badge kind={s.badge} />
            <RenameField
              initial={s.title}
              onCommit={(title) => {
                setRenaming(false);
                void sessions?.rename(s.id, title);
              }}
              onCancel={() => {
                setRenaming(false);
              }}
            />
          </div>
        ) : (
          <button
            type="button"
            className="srow-main"
            aria-current={selected}
            data-session-id={s.id}
            title={`${s.title} · ${status}${s.unread ? " · unread" : ""}${s.localOnly ? " · not saved by Vibe yet" : ""}`}
            onClick={() => void sessions?.open(s.id)}
            onKeyDown={(e) => {
              if (e.key === "F2") {
                e.preventDefault();
                setRenaming(true);
              } else if (e.key === "Delete") {
                e.preventDefault();
                void sessions?.remove(s.id);
              } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
                e.preventDefault();
                const box = e.currentTarget.getBoundingClientRect();
                openMenuAt(box.left + 24, box.bottom);
              }
            }}
          >
            <Badge kind={s.badge} />
            <span className="t">{s.title}</span>
            {s.pinned && (
              <span className="tag pin" role="img" aria-label="Pinned">
                <Icon name="pin" size={12} />
              </span>
            )}
            {s.extraFolders.length > 0 && (
              <span className="tag" title={`Also works in: ${s.extraFolders.join(", ")}`}>
                +{s.extraFolders.length}
                {s.extraFolders.length === 1 ? " folder" : " folders"}
              </span>
            )}
            {node.chip !== null && (
              <span className="tag folder-chip" title={`Works in ${s.cwd ?? ""}`}>
                <Icon name="folder" size={12} />
                {node.chip}
              </span>
            )}
            <VoiceChatTag sessionId={s.id} />
            {s.id === cliId && <span className="tag">Last in CLI</span>}
            {s.localOnly && <span className="tag">local</span>}
            {temporary && (
              <span className="tag" title="Temporary: deleted when you quit Vibe Work">
                temporary
              </span>
            )}
            {s.archived && <span className="tag">archived</span>}
          </button>
        )}
        {s.unread ? <span className="unread" role="img" aria-label="Unread" /> : <span />}
        <button
          type="button"
          className="srow-more"
          aria-label={`Actions for ${s.title}`}
          title={`Actions for ${s.title}`}
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          onClick={(e) => {
            const box = e.currentTarget.getBoundingClientRect();
            openMenuAt(box.left, box.bottom + 2);
          }}
        >
          <Icon name="ellipsis" size={14} />
        </button>
      </div>
      {menu !== null && (
        <RowMenu
          at={menu}
          label={`Actions for ${s.title}`}
          items={items}
          onClose={() => {
            setMenu(null);
          }}
        />
      )}
    </div>
  );
}
