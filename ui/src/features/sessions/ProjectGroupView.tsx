/**
 * One group of the list: a header (name, count, running mark, unread count, actions menu) and its sessions.
 * a project (the user's own group) can be renamed inline (or from the menu), deleted (Delete key or the
 * menu: the window's confirmation, sessions are never deleted), reordered (drag the header, or Move up / Move down),
 * given a main folder (Choose main folder…), and takes dropped sessions. A folder group (before the core filed a
 * session) keeps the old menu: new session here, show folder, trust.
 * Depends on: AppContext, hooks, useGroups, dnd, listDrop (the column decides drops; the group renders the mark),
 * SessionRow, RowMenu, RenameField, TrustDialog, listModel, components/Icon. Called by: SessionList.
 * Invariants: collapsing is per project id (folder key before filing), including during search; "Show folder" and
 * "Trust" need a path; the group never changes data itself (it calls `groups.*` / `sessions.*`); drops are decided by
 * the column (`listDrop.ts`), which reads `data-drop-key` / `data-drop-id`. The compact chat-plus action and menu
 * share the project start path, which selects the new chat, expands its group and focuses its composer.
 */
import { TEXT } from "../../vocabulary";
import { useState } from "react";

import { Icon } from "../../components/Icon";
import { IconButton } from "../../components/IconButton";
import { Walker } from "../../components/Walker";
import { useApp } from "../shell/AppContext";
import { useShellState } from "../shell/hooks";
import { dragProject } from "./dnd";
import { useDropMark } from "./listDrop";
import type { ProjectGroup } from "./listModel";
import { RenameField } from "./RenameField";
import { type MenuItem, RowMenu } from "../../components/RowMenu";
import { SessionRows } from "./SessionRows";
import { TrustDialog } from "./TrustDialog";
import { useGroupActions, useGroups } from "./useGroups";

/** The project ids in the user's order (for "after" drops and Move up/down). */
function useOrder(): readonly string[] {
  return useGroups((g) => g.data?.projects)?.map((p) => p.id) ?? [];
}

/** The full group stays mounted; `shown` supplies filtered content, or null to hide the group. */
export function ProjectGroupView({
  group,
  shown = group,
}: {
  group: ProjectGroup;
  shown?: ProjectGroup | null;
}) {
  const { shell, sessions } = useApp();
  const groups = useGroupActions();
  const collapsed = useShellState((s) => s.collapsed[group.key] === true);
  const renaming = useGroups((g) => group.id !== null && g.renaming === group.id);
  const order = useOrder();
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [trusting, setTrusting] = useState(false);
  const drop = useDropMark(group.key);
  const open = !collapsed;
  const path = group.path;
  const id = group.id;
  const at = id === null ? -1 : order.indexOf(id);

  const newHere = () => {
    if (id !== null) void groups?.startIn(id);
    else if (path !== null) void sessions?.startIn(path);
  };
  const project: MenuItem[] =
    id === null || groups === null
      ? []
      : [
          {
            id: "rename",
            label: "Rename",
            run: () => {
              groups.setRenaming(id);
            },
          },
          {
            id: "folder",
            label: path === null ? "Choose main folder..." : "Change main folder...",
            run: () => void groups.chooseFolder(id),
          },
          {
            id: "up",
            label: "Move up",
            run: () => void groups.shift(id, -1),
            disabled: at <= 0,
            separated: true,
          },
          {
            id: "down",
            label: "Move down",
            run: () => void groups.shift(id, 1),
            disabled: at < 0 || at >= order.length - 1,
          },
        ];
  const items: MenuItem[] = [
    {
      id: "new",
      label: id === null ? "New session here" : `${TEXT.newChat} in this project`,
      run: newHere,
      disabled: id === null && path === null,
    },
    ...project,
    {
      id: "reveal",
      label: "Show folder",
      run: () => void (path !== null && sessions?.revealPath(path)),
      disabled: path === null,
      separated: id !== null,
    },
    {
      id: "trust",
      label: "Trust this folder's hooks, agents and plugins...",
      run: () => {
        setTrusting(true);
      },
      disabled: path === null,
      separated: id === null,
    },
    ...(id === null || groups === null
      ? []
      : [
          {
            id: "delete",
            label: "Delete project...",
            run: () => {
              groups.askDelete(id);
            },
            danger: true,
            separated: true,
          },
        ]),
  ];

  return (
    <div
      data-item-id={group.key}
      className="proj"
      hidden={shown === null}
      data-open={open}
      data-project={group.key}
      data-project-id={id ?? undefined}
      data-drop-key={group.key}
      data-drop-id={id ?? ""}
      data-drop={drop}
    >
      <div
        data-testid="sessions.project-group-view-proj-bar"
        className="proj-bar"
        draggable={id !== null && groups !== null && !renaming}
        onDragStart={(e) => {
          if (id !== null) dragProject(e.dataTransfer, id);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
      >
        {renaming && id !== null ? (
          <div className="proj-h proj-rename">
            <span className="fold" aria-hidden="true">
              <Icon name="folder" size={16} />
            </span>
            <RenameField
              initial={group.name}
              label="Rename project"
              onCommit={(name) => {
                groups?.setRenaming(null);
                void groups?.rename(id, name);
              }}
              onCancel={() => {
                groups?.setRenaming(null);
              }}
            />
          </div>
        ) : (
          <button
            data-testid="sessions.project-group-view-toggle-project"
            type="button"
            className="proj-h"
            aria-expanded={open}
            title={
              id === null
                ? (path ?? "Sessions without a folder")
                : `${group.name}${path === null ? " (no main folder)" : ` · ${path}`}`
            }
            onClick={() => {
              shell.actions.toggleProject(group.key);
            }}
            onKeyDown={(e) => {
              if (id === null || groups === null) return;
              if (e.key === "F2") {
                e.preventDefault();
                groups.setRenaming(id);
              } else if (e.key === "Delete") {
                e.preventDefault();
                groups.askDelete(id);
              } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
                e.preventDefault();
                const box = e.currentTarget.getBoundingClientRect();
                setMenu({ x: box.left + 24, y: box.bottom });
              }
            }}
          >
            <span className="caret" aria-hidden="true">
              <Icon name="chevron-right" size={12} />
            </span>
            <span className="fold" aria-hidden="true">
              <Icon name="folder" size={16} />
            </span>
            <span className="pname">{group.name}</span>
            <span className="cnt">
              {shown?.running && (
                <span className="sb run" role="img" aria-label="A session is running">
                  <Walker size="badge" />
                </span>
              )}
              {(shown?.unread ?? 0) > 0 && (
                <span
                  className="unread"
                  role="img"
                  aria-label={`${String(shown?.unread)} unread`}
                />
              )}
              {shown?.count ?? 0}
            </span>
          </button>
        )}
        {id !== null && (
          <IconButton
            data-testid="sessions.project-group-view-proj-new-chat"
            icon="message-square-plus"
            iconSize={14}
            label={`${TEXT.newChat} in ${group.name}`}
            className="proj-new-chat"
            disabled={groups === null}
            onClick={newHere}
          />
        )}
        <button
          data-testid="sessions.project-group-view-get-bounding-client-rect"
          type="button"
          className="srow-more"
          title="Project actions"
          aria-label={`Actions for ${group.name}`}
          aria-haspopup="menu"
          aria-expanded={menu !== null}
          onClick={(e) => {
            const box = e.currentTarget.getBoundingClientRect();
            setMenu({ x: box.left, y: box.bottom + 2 });
          }}
        >
          <Icon name="ellipsis" size={14} />
        </button>
      </div>
      {menu !== null && (
        <RowMenu
          at={menu}
          label={`Actions for ${group.name}`}
          items={items}
          onClose={() => {
            setMenu(null);
          }}
        />
      )}
      <div className="proj-sessions" hidden={!open}>
        <SessionRows rows={group.rows} shown={shown?.rows ?? []} />
        {shown?.rows.length === 0 && id !== null && (
          <div className="proj-empty">
            <span>
              No sessions here. Drag sessions here, or{" "}
              <button
                data-testid="sessions.project-group-view-link-btn"
                type="button"
                className="link-btn"
                onClick={newHere}
              >
                start one
              </button>
              .
            </span>
            {path === null && (
              <button
                data-testid="sessions.project-group-view-choose-folder"
                type="button"
                className="link-btn"
                onClick={() => void groups?.chooseFolder(id)}
              >
                Choose main folder...
              </button>
            )}
          </div>
        )}
      </div>
      {trusting && path !== null && (
        <TrustDialog
          cwd={path}
          onClose={() => {
            setTrusting(false);
          }}
        />
      )}
    </div>
  );
}
