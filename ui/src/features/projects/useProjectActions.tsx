/**
 * Project management controls shared by overview cards and the detail pane: the visible menu button,
 * right-click menu, and inline rename field. Reuses the shared RowMenu and RenameField and
 * calls the existing projects/groups controllers; deletion opens the shared DeleteProjectDialog.
 * Depends on: AppContext, useGroups, RowMenu, RenameField, Icon. Called by: ProjectCardView, ProjectDetail.
 * Invariants: deleting only unfiles conversations; folder actions use the project's main folder;
 * editing is local to this surface so the session list cannot steal the rename field's focus.
 */
import { TEXT } from "../../vocabulary";
import { type MouseEvent, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { RenameField } from "../sessions/RenameField";
import { type MenuItem, RowMenu } from "../../components/RowMenu";
import { useGroups } from "../sessions/useGroups";
import { useApp } from "../shell/AppContext";

/**
 * Returns menu, button, context-menu handler and inline rename field for `projectId` and its current `title`.
 * Actions call the existing controllers; Escape cancels edits and returns focus to the action button.
 */
export function useProjectActions(projectId: string, title: string) {
  const { projects } = useApp();
  const project = useGroups((g) => g.data?.projects.find((p) => p.id === projectId));
  const button = useRef<HTMLButtonElement>(null);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const finishRename = () => {
    setRenaming(false);
    button.current?.focus();
  };
  const items: MenuItem[] = [
    {
      id: "settings",
      label: "Project settings",
      run: () => {
        projects?.openSettings(projectId);
      },
    },
    {
      id: "rename",
      label: "Rename",
      run: () => {
        setRenaming(true);
      },
    },
    {
      id: "folder",
      label: "Change main folder...",
      run: () => void projects?.chooseFolder(projectId),
    },
    {
      id: "reveal",
      label: "Open folder",
      run: () => void projects?.reveal(projectId),
      disabled: project?.mainFolder == null,
    },
    {
      id: "new",
      label: `${TEXT.newChat} in this project`,
      run: () => void projects?.newSession(projectId),
    },
    {
      id: "delete",
      label: "Delete project...",
      run: () => {
        projects?.groups.askDelete(projectId);
      },
      danger: true,
      separated: true,
    },
  ];

  return {
    onContextMenu: (event: MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (project !== undefined) {
        button.current?.focus();
        setAt({ x: event.clientX, y: event.clientY });
      }
    },
    button: (
      <button
        data-testid="projects.use-project-actions-stop-propagation"
        ref={button}
        type="button"
        className="pj-actions"
        title="Project actions"
        aria-label={`Actions for ${title}`}
        aria-haspopup="menu"
        aria-expanded={at !== null}
        disabled={project === undefined}
        onClick={(event) => {
          event.stopPropagation();
          const box = event.currentTarget.getBoundingClientRect();
          setAt({ x: box.left, y: box.bottom + 2 });
        }}
      >
        <Icon name="ellipsis" size={16} />
      </button>
    ),
    renameField: renaming ? (
      <RenameField
        initial={title}
        label="Rename project"
        onCommit={(name) => {
          finishRename();
          void projects?.groups.rename(projectId, name);
        }}
        onCancel={finishRename}
      />
    ) : null,
    menu:
      at === null ? null : (
        <div
          data-testid="projects.use-project-actions-stop-propagation-2"
          className="pj-action-menu"
          onClick={(event) => {
            event.stopPropagation();
          }}
        >
          <RowMenu
            at={at}
            label={`Actions for ${title}`}
            items={items}
            onClose={() => {
              setAt(null);
            }}
          />
        </div>
      ),
  };
}
