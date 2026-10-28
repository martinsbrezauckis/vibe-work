/**
 * "Move to project…": the keyboard and pointer way to file sessions without dragging. A menu of the
 * projects in the user's order, "Not in a project" and "New project…", opened from a row's menu or the palette.
 * Depends on: RowMenu, useGroups. Called by: SessionList (open while `groups.mover` is set).
 * Invariants: the project every moved session is already in is shown disabled ("(here)"); the menu is the same
 * `RowMenu` as the row's (arrow keys, Enter, Escape, focus returns).
 */
import { type MenuItem, RowMenu } from "../../components/RowMenu";
import { useGroupActions, useGroups } from "./useGroups";

/** The menu while `groups.mover` is set. */
export function MoveMenu() {
  const groups = useGroupActions();
  const mover = useGroups((g) => g.mover);
  const data = useGroups((g) => g.data);
  if (mover === null || data === null || groups === null) return null;
  const all = (projectId: string | null) =>
    mover.sessionIds.every((id) => data.members[id] === projectId);
  const items: MenuItem[] = [
    ...data.projects.map((p) => ({
      id: `to-${p.id}`,
      label: all(p.id) ? `${p.name} (here)` : p.name,
      disabled: all(p.id),
      run: () => void groups.move(mover.sessionIds, p.id),
    })),
    {
      id: "to-none",
      label: "Not in a project",
      disabled: all(null),
      separated: data.projects.length > 0,
      run: () => void groups.move(mover.sessionIds, null),
    },
    {
      id: "to-new",
      label: "New project…",
      run: () => void groups.createWith(mover.sessionIds),
    },
  ];
  return (
    <RowMenu
      at={mover.at}
      label="Move to project"
      items={items}
      onClose={() => {
        groups.closeMover();
      }}
    />
  );
}
