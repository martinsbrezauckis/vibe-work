/**
 * Opens the existing copies inventory in the welcome view, from the sessions footer or Settings.
 * The bell's Sessions action also reaches this list by returning to the welcome view.
 * Depends on: AppContext and home navigation. Keeps the new-chat draft and chosen folder.
 */
import type { App } from "../shell/AppContext";
import { goHome } from "../shell/home";
import { focusWhenMounted } from "../../hooks/focusWhenMounted";

/** Opens and focuses the copies list after refreshing its count. */
export async function openCopies(app: App): Promise<void> {
  goHome(app);
  app.shell.actions.selectSession(null);
  await app.worktrees?.loadOrphans();
  focusWhenMounted({
    find: () => document.querySelector<HTMLElement>('[data-worktree="orphans"]'),
    center: true,
    preventScroll: true,
  });
}
