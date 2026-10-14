/**
 * Hooks of the user's project groups for the list components: a selector over the groups store and the
 * actions. Without a core the state is a fixed empty one and the actions are `null`.
 * Depends on: AppContext, createStore, state/projectGroups. Called by: SessionList, ProjectGroupView, SessionRow,
 * the move menu and the delete dialog.
 */
import { createStore, useStore } from "../../state/createStore";
import type { GroupsState, ProjectGroups } from "../../state/projectGroups";
import { useApp } from "../shell/AppContext";

const NONE = createStore<GroupsState>({
  data: null,
  error: null,
  renaming: null,
  creating: false,
  mover: null,
  deleting: null,
});

/** Subscribes to `select(state)` of the groups store (see `useStore` for the selector rule). */
export function useGroups<S>(select: (state: GroupsState) => S): S {
  const { projects } = useApp();
  return useStore(projects?.groups.store ?? NONE, select);
}

/** The groups' actions, `null` without a core. */
export function useGroupActions(): ProjectGroups | null {
  return useApp().projects?.groups ?? null;
}
