/**
 * Resolves the active saved project id from the shell selection, explicit project pick, or New chat folder.
 * Depends on: AppContext types. Called by: git-browser/register, git-browser/GitBrowserView.
 */
import type { App } from "../shell/AppContext";

/** Returns the active saved project id for the git browser panel, or `null`. */
export function currentProject(app: App): string | null {
  const groups = app.projects?.groups.store.get().data;
  if (groups === null || groups === undefined) return null;
  const shell = app.shell.store.get();
  if (shell.selected !== null) return groups.members[shell.selected] ?? null;
  const selected = app.projects?.store.get().selected;
  if (
    selected !== null &&
    selected !== undefined &&
    groups.projects.some((p) => p.id === selected && p.mainFolder !== null)
  )
    return selected;
  const folder = shell.newFolder;
  return groups.projects.find((p) => p.mainFolder !== null && p.mainFolder === folder)?.id ?? null;
}
