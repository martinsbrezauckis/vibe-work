/**
 * The "New project" name field at the top of the projects: Enter or leaving the field with a name
 * creates a name-only project (its main folder can be chosen afterwards from its menu or its empty state); Escape or
 * an empty name cancels.
 * Depends on: RenameField, useGroups, components/Icon. Called by: SessionList (while `groups.creating`).
 * Invariants: creates at most once per opening (RenameField commits once).
 */
import { Icon } from "../../components/Icon";
import { RenameField } from "./RenameField";
import { useGroupActions } from "./useGroups";

/** The field. */
export function NewProjectField() {
  const groups = useGroupActions();
  if (groups === null) return null;
  return (
    <div className="proj proj-new">
      <div className="proj-h proj-rename">
        <span className="fold" aria-hidden="true">
          <Icon name="folder" size={16} />
        </span>
        <RenameField
          initial=""
          label="New project name"
          placeholder="Project name"
          onCommit={(name) => {
            groups.setCreating(false);
            void groups.create(name);
          }}
          onCancel={() => {
            groups.setCreating(false);
          }}
        />
      </div>
    </div>
  );
}
