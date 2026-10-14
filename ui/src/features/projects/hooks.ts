/**
 * Hooks of the Projects overview: the page state from the projects controller. Without a core the state
 * is a fixed closed one.
 * Depends on: AppContext, createStore, state/projects. Called by: the rail and the content area.
 */
import { createStore, useStore } from "../../state/createStore";
import type { ProjectsState } from "../../state/projects";
import { useApp } from "../shell/AppContext";

const NONE = createStore<ProjectsState>({
  open: false,
  cards: null,
  loading: false,
  error: null,
  elapsedMs: null,
  hubs: {},
  filter: "all",
  query: "",
  selected: null,
  settings: null,
});

/** Subscribes to `select(state)` of the page (see `useStore` for the selector rule). */
export function useProjects<S>(select: (state: ProjectsState) => S): S {
  const { projects } = useApp();
  return useStore(projects?.store ?? NONE, select);
}
