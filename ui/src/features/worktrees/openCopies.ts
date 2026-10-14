/**
 * Opens the existing copies inventory in the welcome view, from the sessions footer or a bell action.
 * Depends on: AppContext and home navigation. Keeps the new-chat draft and chosen folder.
 */
import type { App } from "../shell/AppContext";
import { goHome } from "../shell/home";

/** Opens and focuses the copies list after refreshing its count. */
export async function openCopies(app: App): Promise<void> {
  goHome(app);
  app.shell.actions.selectSession(null);
  await app.worktrees?.loadOrphans();
  requestAnimationFrame(() => {
    const list = document.querySelector<HTMLElement>('[data-worktree="orphans"]');
    list?.scrollIntoView({ block: "center" });
    list?.focus({ preventScroll: true });
  });
}
