/**
 * Pure helpers of the git views: when the branch bar shows, what its buttons may do, and the PR
 * card's words. No state, no core calls.
 * Depends on: protocol types. Called by: the views in this folder, the `ui/git` route, register (tab visibility).
 */
import type { GitStatusView, PrCard, PrStatusView } from "../../protocol/core";

/** A discovered repository keeps its branch bar visible, including clean and blocked states. */
export function barShown(status: GitStatusView | undefined): boolean {
  return status !== undefined && status.available && status.repos.length > 0;
}

/** The current branch name, or null when detached. */
export function branchOf(status: GitStatusView): string | null {
  const head = status.state?.head;
  if (head === undefined) return null;
  return head.kind === "detached" ? null : head.name;
}

/** Why Commit is not possible now (null: it is). */
export function commitBlocked(status: GitStatusView): string | null {
  if (status.running) return "Vibe is still working. Commit when the turn has finished.";
  if (status.blocker !== null) return status.blocker;
  if ((status.uncommitted ?? 0) === 0) return "Nothing to commit.";
  return null;
}

/** Why Push is not possible now (null: it is). */
export function pushBlocked(status: GitStatusView): string | null {
  const state = status.state;
  if (state === null || branchOf(status) === null) return "Not on a branch.";
  if (state.commit === null) return "No commit yet.";
  if (!state.remotes.some((r) => r.pushable))
    return "Not connected to GitHub or another server yet.";
  if (state.upstream !== null && state.upstream.ahead === 0) return "Nothing new to push.";
  return null;
}

/** Why Create PR is not possible now (null: it is). */
export function prBlocked(status: GitStatusView): string | null {
  const state = status.state;
  if (state === null || branchOf(status) === null) return "Not on a branch.";
  if (state.commit === null) return "No commit yet.";
  if (state.remotes.length === 0) return "not connected to GitHub or another server yet.";
  if (!state.remotes.some((r) => r.pushable && r.forge !== null && r.forge.kind !== "unknown"))
    return "needs a GitHub repository";
  if (status.pr !== null && status.pr.state === "open")
    return "A pull request is open for this branch.";
  return null;
}

/** `+N −M` of the branch against its base (committed work), or null without a base or without commits on it. */
export function baseStat(status: GitStatusView): string | null {
  const base = status.state?.base;
  if (base === null || base === undefined || base.ahead === 0) return null;
  return `+${String(base.added)} −${String(base.removed)}`;
}

/** The card's one-line summary of the checks. */
export function checksText(card: PrCard): string {
  const { passed, failed, pending } = card.checks;
  if (passed + failed + pending === 0) return "No checks";
  const parts: string[] = [];
  if (failed > 0) parts.push(`${String(failed)} failing`);
  if (pending > 0) parts.push(`${String(pending)} running`);
  if (passed > 0) parts.push(`${String(passed)} passed`);
  return `Checks: ${parts.join(", ")}`;
}

/** The card's review line. */
export function reviewText(card: PrCard): string {
  switch (card.review) {
    case "approved":
      return "Approved";
    case "changesRequested":
      return "Changes requested";
    case "required":
      return "Review required";
    default:
      return "No review yet";
  }
}

/** The card's state word. */
export function stateText(card: PrCard): string {
  if (card.state === "merged") return "Merged";
  if (card.state === "closed") return "Closed";
  return card.draft ? "Draft" : "Open";
}

/** The card's merge line. */
export function mergeText(card: PrCard): string {
  if (card.state !== "open") return "";
  return card.mergeable === "conflicting"
    ? "Conflicts with the base"
    : card.mergeable === "yes"
      ? "Can merge"
      : "Mergeability not known yet";
}

/** "checked at 14:05" for the card (empty before the first check). */
export function fetchedText(view: PrStatusView): string {
  if (view.fetchedAtMs === 0) return "";
  const at = new Date(view.fetchedAtMs).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
  return `checked at ${at}`;
}

/**
 * A forge or CI address (from `gh`, GitHub's API, check runs: data a third party controls) when it is an
 * absolute `https:` URL, with its host for the label; `null` for anything else (`file:`, `http:`, `data:`,
 * `javascript:`, custom schemes such as `ms-msdt:` or `vscode:`). The core's `openLink` checks again in Rust.
 */
export function httpsLink(text: string | null | undefined): { url: string; host: string } | null {
  if (text === null || text === undefined) return null;
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  return url.protocol === "https:" && url.host !== "" ? { url: url.href, host: url.host } : null;
}
