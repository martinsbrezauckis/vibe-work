/** Git browser rows and safe next steps. Depends on existing git view gates; called by GitBrowser. */
import { TEXT } from "../../vocabulary";
import type { BrowserStatus, GitStatusView } from "../../protocol/core";
import { commitBlocked, prBlocked, pushBlocked } from "../git/model";

export type BrowserAction =
  "commit" | "push" | "pr" | "bring" | "new-chat" | "new-task" | "open-chat";
export type BrowserFile = { path: string; status: string };

/** File labels use everyday words; the wire status stays available to Debug clients. */
export function fileWords(status: string): string {
  return status === "untracked" || status === "added"
    ? "New file"
    : status === "conflict"
      ? "Needs conflict resolution"
      : status === "deleted"
        ? "Deleted"
        : status === "renamed"
          ? "Renamed"
          : "Changed";
}
export type BrowserTarget = {
  id: string;
  kind?: "repository" | "copy";
  name: string;
  path: string;
  status: GitStatusView | null;
  /** Read-only status is available even when no session admits Git actions. */
  observed?: BrowserStatus | null;
  files: BrowserFile[];
  words?: string;
  branch?: string | null;
  error?: string | null;
  busy?: boolean;
  actionReason?: string | null;
  restricted?: boolean;
  sessionId?: string | null;
  repo?: string | null;
};
export type BrowserRepo = BrowserTarget & { copies: BrowserTarget[] };
export type BrowserStep = { action: BrowserAction; label: string; reason: string | null };

/** Unknown and blocked state stays visible; it never becomes a clean repository. */
export function stateWords(target: BrowserTarget): string {
  if (target.kind === "copy" && target.restricted)
    return target.observed?.words ?? "Vibe Work keeps this copy in its own folder.";
  if (target.error) return target.error;
  if (target.words) return target.words;
  const status = target.status;
  if (!status?.available || status.state === null)
    return status?.reason ?? "Git status unavailable.";
  const parts: string[] = [];
  if (status.uncommitted === null) parts.push("Changed file count unavailable");
  else if (status.uncommitted > 0)
    parts.push(
      `${String(status.uncommitted)} changed ${status.uncommitted === 1 ? "file" : "files"}`,
    );
  const upstream = status.state.upstream;
  if (upstream === null) parts.push("Not published yet");
  else {
    if (upstream.ahead > 0)
      parts.push(
        `${String(upstream.ahead)} ${upstream.ahead === 1 ? "commit" : "commits"} not sent`,
      );
    if (upstream.behind > 0)
      parts.push(
        `${String(upstream.behind)} new ${upstream.behind === 1 ? "commit" : "commits"} on remote`,
      );
  }
  if (status.blocker) parts.push(status.blocker);
  if (status.running) parts.push("Vibe is working");
  return parts.length > 0 ? parts.join(" · ") : "Up to date";
}

/** Common admission gate used by primary and overflow actions. */
function admitted(target: BrowserTarget): string | null {
  if (target.restricted) return "This folder is outside the allowed project scope.";
  if (target.error) return target.error;
  if (!target.sessionId || target.repo === null || target.repo === undefined)
    return "Open a chat in this folder to use Git actions.";
  if (target.busy) return "Another git action is in progress.";
  if (target.actionReason) return target.actionReason;
  const status = target.status;
  if (!status?.available || status.state === null)
    return status?.reason ?? "Git status unavailable.";
  if (status.running) return "Vibe is still working. Wait for the turn to finish.";
  if (status.blocker !== null) return status.blocker;
  if (status.uncommitted === null) return "Changed file count unavailable.";
  return null;
}

/** The existing dialog performs the final preview and confirmation; no write happens here. */
export function actionStep(target: BrowserTarget, action: BrowserAction): BrowserStep {
  const status = target.status;
  let label: string;
  let reason = admitted(target);
  if (action === "open-chat") {
    label = "Open its chat";
    reason = target.sessionId ? null : "This copy has no session.";
  } else if (action === "new-chat" || action === "new-task") {
    label = action === "new-chat" ? `${TEXT.newChat} here` : "New task here";
    reason =
      target.restricted && target.kind !== "copy"
        ? "This folder is outside the allowed project scope."
        : null;
  } else if (action === "commit") {
    label = "Commit";
    reason ??= status === null ? "Git status unavailable." : commitBlocked(status);
  } else if (action === "push") {
    const upstream = status?.state ? status.state.upstream : target.observed?.upstream;
    const ahead = status?.state?.upstream?.ahead ?? target.observed?.ahead ?? 0;
    label = upstream == null ? "Publish branch" : `Push (${String(ahead)})`;
    reason ??= status === null ? "Git status unavailable." : pushBlocked(status);
    if ((status?.state?.upstream?.behind ?? target.observed?.behind ?? 0) > 0)
      reason ??= "The remote has new commits. Review them before pushing.";
  } else if (action === "pr") {
    label = "Create PR";
    reason ??= status === null ? "Git status unavailable." : prBlocked(status);
    if (status?.state?.upstream == null || (status.toPush ?? 0) > 0)
      reason ??= "Publish this branch's commits before creating a pull request.";
  } else {
    const base = status?.state?.base;
    label = `Bring into ${base?.name ?? "main"}`;
    if (status?.location !== "copy" || status.repo !== "." || base == null)
      reason ??= "Only a session copy with a known base can bring commits in.";
    else if (base.ahead === 0) reason ??= `${base.name} already has these commits.`;
    else if ((status.uncommitted ?? 0) > 0) reason ??= "Commit the changes first.";
    // The existing Bring dialog performs the authoritative dry run and moved-base check.
  }
  return { action, label, reason };
}

/** Exactly one suggested action, with a truthful reason when its existing route cannot run. */
export function nextStep(target: BrowserTarget): BrowserStep {
  if (target.kind === "copy" && target.restricted)
    return actionStep(target, target.sessionId ? "open-chat" : "new-chat");
  const status = target.status;
  const dirty =
    status?.uncommitted ??
    (target.observed === null || target.observed === undefined
      ? 0
      : target.observed.changedFiles + target.observed.untrackedFiles);
  if (dirty > 0) return actionStep(target, "commit");
  const upstream = status?.state ? status.state.upstream : target.observed?.upstream;
  if (upstream == null) return actionStep(target, "push");
  const ahead = status?.state?.upstream?.ahead ?? target.observed?.ahead ?? 0;
  if (ahead > 0) return actionStep(target, "push");
  if (
    status?.location === "copy" &&
    status.repo === "." &&
    status.state?.base !== null &&
    status.state?.base !== undefined &&
    status.pr?.state === "open"
  )
    return actionStep(target, "bring");
  return actionStep(target, "pr");
}
