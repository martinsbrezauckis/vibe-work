/** Maps core browser snapshots to tree rows. Depends on the generated browser contract; called by register. */
import type { GitBrowserNode, GitBrowserSnapshot } from "../../protocol/core";
import type { BrowserRepo, BrowserTarget } from "./model";

function targetOf(node: GitBrowserNode): BrowserTarget {
  return {
    id: node.id,
    kind: node.kind === "copy" ? "copy" : "repository",
    name: node.name,
    path: node.path,
    branch: node.branch,
    status: node.gitStatus,
    observed: node.status,
    files: node.files,
    ...(node.status === null ? {} : { words: node.status.words }),
    error: node.restricted,
    restricted: node.outsideProject,
    sessionId: node.sessionId,
    repo: node.repo,
  };
}

export function reposOf(snapshot: GitBrowserSnapshot | null): BrowserRepo[] {
  if (snapshot === null) return [];
  const repos: BrowserRepo[] = [];
  const byId = new Map<string, BrowserRepo>();
  for (const node of snapshot.nodes) {
    if (node.kind === "copy") continue;
    const repo: BrowserRepo = { ...targetOf(node), copies: [] };
    repos.push(repo);
    byId.set(node.id, repo);
  }
  for (const node of snapshot.nodes) {
    if (node.kind !== "copy") continue;
    const parent = node.parentId === null ? undefined : byId.get(node.parentId);
    if (parent !== undefined) parent.copies.push(targetOf(node));
    else repos.push({ ...targetOf(node), copies: [] });
  }
  return repos;
}
