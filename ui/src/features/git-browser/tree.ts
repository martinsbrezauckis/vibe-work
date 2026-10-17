/** Flattens expanded repository/copy/file nodes for virtua. Depends on browser model; called by GitBrowser. */
import type { BrowserRepo, BrowserTarget } from "./model";

export type BrowserRow = {
  id: string;
  parent: string | null;
  level: number;
  position: number;
  size: number;
  kind: "repository" | "copies" | "copy" | "file";
  name: string;
  target: BrowserTarget;
  file?: { path: string; status: string };
  expandable: boolean;
};

export function treeRows(repos: BrowserRepo[], collapsed: ReadonlySet<string>): BrowserRow[] {
  const rows: BrowserRow[] = [];
  function files(
    target: BrowserTarget,
    parent: string,
    level: number,
    offset = 0,
    size = target.files.length,
  ) {
    target.files.forEach((file, index) =>
      rows.push({
        id: `${parent}:file:${file.path}`,
        parent,
        level,
        position: index + 1 + offset,
        size,
        kind: "file",
        name: file.path,
        target,
        file,
        expandable: false,
      }),
    );
  }
  repos.forEach((repo, index) => {
    rows.push({
      id: repo.id,
      parent: null,
      level: 1,
      position: index + 1,
      size: repos.length,
      kind: repo.kind === "copy" ? "copy" : "repository",
      name: repo.name,
      target: repo,
      expandable: repo.copies.length + repo.files.length > 0,
    });
    if (collapsed.has(repo.id)) return;
    const count = repo.files.length + (repo.copies.length > 0 ? 1 : 0);
    if (repo.copies.length > 0) {
      const group = `${repo.id}:copies`;
      rows.push({
        id: group,
        parent: repo.id,
        level: 2,
        position: 1,
        size: count,
        kind: "copies",
        name: "Copies",
        target: repo,
        expandable: true,
      });
      if (!collapsed.has(group))
        repo.copies.forEach((copy, copyIndex) => {
          rows.push({
            id: copy.id,
            parent: group,
            level: 3,
            position: copyIndex + 1,
            size: repo.copies.length,
            kind: "copy",
            name: copy.name,
            target: copy,
            expandable: copy.files.length > 0,
          });
          if (!collapsed.has(copy.id)) files(copy, copy.id, 4);
        });
    }
    files(repo, repo.id, 2, repo.copies.length > 0 ? 1 : 0, count);
  });
  return rows;
}
