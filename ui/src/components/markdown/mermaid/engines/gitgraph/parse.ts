/**
 * GitGraph diagram parser. Follows Mermaid's gitgraph page: `gitGraph [LR:|TB:|BT:]`, `commit`,
 * `branch` (with optional `order:`), `checkout`/`switch`, `merge` (with optional `id`/`tag`/`type`),
 * and `cherry-pick` enforcing all five parent/branch preconditions. Refuses directives/styles/links.
 */
import { linesOf } from "../../detect";
import { MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** Commit visual style kinds documented by Mermaid's gitGraph syntax. */
export type GitCommitType = "NORMAL" | "REVERSE" | "HIGHLIGHT";

/** Graph progression directions supported by Mermaid's gitGraph syntax. */
export type GitDirection = "LR" | "TB" | "BT";

/** A branch lane in resolved display order with its colour index and optional origin commit. */
export interface GitBranch {
  name: string;
  order: number | undefined;
  createdIndex: number;
  colorIndex: number;
  parentCommitId?: string;
}

/** A single commit (regular, merge, or cherry-pick) on a branch. */
export interface GitCommit {
  seq: number;
  id: string;
  customId: boolean;
  type: GitCommitType;
  tag?: string;
  branch: string;
  parents: string[];
  parentSeqs: number[];
  isMerge: boolean;
  cherryPickFrom?: string;
  cherryPickFromSeq?: number;
}

/** Parsed gitGraph diagram ready for layout and drawing. */
export interface GitGraphModel {
  direction: GitDirection;
  branches: GitBranch[];
  commits: GitCommit[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style)\b|:::/i;
const IGNORED = /^(?:accTitle|accDescr)\b/i;

function readBranchToken(input: string, command: string): { name: string; rest: string } {
  const s = input.trim();
  if (!s) throw new DiagramInputError(`Missing branch name after ${command}`);
  if (s[0] === '"' || s[0] === "'") {
    const q = s[0];
    let i = 1;
    let out = "";
    let closed = false;
    while (i < s.length) {
      const ch = s[i];
      if (ch === "\\" && i + 1 < s.length) {
        out += s[i + 1] ?? "";
        i += 2;
        continue;
      }
      if (ch === q) {
        closed = true;
        i++;
        break;
      }
      out += ch ?? "";
      i++;
    }
    if (!closed || !out.trim()) {
      throw new DiagramInputError(`Invalid quoted branch name after ${command}`);
    }
    return { name: out, rest: s.slice(i).trim() };
  }
  const match = /^([^\s:]+)(.*)$/.exec(s);
  if (!match || !match[1]) {
    throw new DiagramInputError(`Invalid branch name after ${command}`);
  }
  return { name: match[1], rest: (match[2] ?? "").trim() };
}

function parseAttributes(input: string, allowed: ReadonlySet<string>): Record<string, string> {
  const attrs: Record<string, string> = {};
  let i = 0;
  const s = input.trim();
  while (i < s.length) {
    while (i < s.length && /\s/.test(s[i] ?? "")) i++;
    if (i >= s.length) break;
    const keyMatch = /^([a-zA-Z]+)\s*:/.exec(s.slice(i));
    if (!keyMatch || !keyMatch[1]) {
      throw new DiagramInputError(`Unreadable gitGraph attribute: ${s.slice(i, i + 30)}`);
    }
    const key = keyMatch[1].toLowerCase();
    if (!allowed.has(key)) {
      throw new DiagramInputError(`Unsupported gitGraph attribute: ${key}`);
    }
    i += keyMatch[0].length;
    while (i < s.length && /\s/.test(s[i] ?? "")) i++;
    if (i >= s.length) {
      throw new DiagramInputError(`Missing value for gitGraph attribute: ${key}`);
    }
    let val = "";
    const ch = s[i];
    if (ch === '"' || ch === "'") {
      const q = ch;
      i++;
      let closed = false;
      while (i < s.length) {
        const cur = s[i];
        if (cur === "\\" && i + 1 < s.length) {
          val += s[i + 1] ?? "";
          i += 2;
          continue;
        }
        if (cur === q) {
          closed = true;
          i++;
          break;
        }
        val += cur ?? "";
        i++;
      }
      if (!closed) throw new DiagramInputError(`Unterminated string in gitGraph ${key}`);
    } else {
      const valMatch = /^([^\s]+)/.exec(s.slice(i));
      if (!valMatch || !valMatch[1]) {
        throw new DiagramInputError(`Invalid value for gitGraph ${key}`);
      }
      val = valMatch[1];
      i += val.length;
    }
    attrs[key] = val;
  }
  return attrs;
}

function parseCommitType(raw: string | undefined): GitCommitType {
  if (raw === undefined) return "NORMAL";
  const upper = raw.toUpperCase();
  if (upper === "NORMAL" || upper === "REVERSE" || upper === "HIGHLIGHT") return upper;
  throw new DiagramInputError(`Unknown gitGraph commit type: ${raw.slice(0, 20)}`);
}

const COMMIT_ATTRS = new Set(["id", "type", "tag"]);
const BRANCH_ATTRS = new Set(["order"]);
const CHERRY_ATTRS = new Set(["id", "parent", "tag"]);

/** Parse `gitGraph` source (text or prepared lines); throws DiagramInputError on syntax/git errors. */
export function parseGitGraph(input: string | PreparedSource): GitGraphModel {
  const [header = "", ...body] = linesOf(input, "gitgraph");
  const headMatch = /^gitGraph(?:\s+(LR|TB|BT))?\s*:?\s*$/i.exec(header.trim());
  if (!headMatch) {
    throw new DiagramInputError(`Unreadable gitGraph header: ${header.trim().slice(0, 40)}`);
  }
  const dirRaw = (headMatch[1] ?? "LR").toUpperCase();
  const direction: GitDirection = dirRaw === "TB" ? "TB" : dirRaw === "BT" ? "BT" : "LR";

  interface BranchState {
    branch: GitBranch;
    head: string | undefined;
    headSeq: number | undefined;
  }

  const branchStates = new Map<string, BranchState>();
  const mainBranch: GitBranch = {
    name: "main",
    order: 0,
    createdIndex: 0,
    colorIndex: 0,
  };
  branchStates.set("main", { branch: mainBranch, head: undefined, headSeq: undefined });
  let currentBranch = "main";
  const commits: GitCommit[] = [];
  const commitsById = new Map<string, GitCommit>();
  let totalEdges = 0;
  let inAccDescrBlock = false;

  const pushCommit = (commit: GitCommit): void => {
    commits.push(commit);
    checkCount(commits.length, MAX_NODES, "commits");
    commitsById.set(commit.id, commit);
    const state = branchStates.get(commit.branch);
    if (state) {
      state.head = commit.id;
      state.headSeq = commit.seq;
    }
  };

  for (const raw of body) {
    const line = raw.trim();
    if (inAccDescrBlock) {
      if (line.includes("}")) inAccDescrBlock = false;
      continue;
    }
    if (/^accDescr\s*\{/i.test(line)) {
      if (!line.includes("}")) inAccDescrBlock = true;
      continue;
    }
    if (IGNORED.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    const cmdMatch = /^(commit|branch|checkout|switch|merge|cherry-pick)(?:\s+(.*))?$/i.exec(line);
    if (!cmdMatch || !cmdMatch[1]) {
      throw new DiagramInputError(`Unreadable gitGraph line: ${line.slice(0, 40)}`);
    }
    const cmd = cmdMatch[1].toLowerCase();
    const rest = (cmdMatch[2] ?? "").trim();

    if (cmd === "commit") {
      const attrs = parseAttributes(rest, COMMIT_ATTRS);
      const state = branchStates.get(currentBranch);
      if (!state) throw new DiagramInputError("Missing current branch");
      const parents = state.head !== undefined ? [state.head] : [];
      const parentSeqs = state.headSeq !== undefined ? [state.headSeq] : [];
      totalEdges += parents.length;
      checkCount(totalEdges, MAX_EDGES, "edges");
      const seq = commits.length;
      const customId = attrs.id !== undefined;
      const id = attrs.id ?? `${String(state.branch.colorIndex)}-${String(seq + 1)}`;
      if (customId && id === "") throw new DiagramInputError("Commit id cannot be empty");
      pushCommit({
        seq,
        id,
        customId,
        type: parseCommitType(attrs.type),
        ...(attrs.tag !== undefined ? { tag: attrs.tag } : {}),
        branch: currentBranch,
        parents,
        parentSeqs,
        isMerge: false,
      });
      continue;
    }

    if (cmd === "branch") {
      const { name, rest: attrRest } = readBranchToken(rest, "branch");
      if (branchStates.has(name)) {
        throw new DiagramInputError(`Branch already exists: ${name}`);
      }
      const attrs = parseAttributes(attrRest, BRANCH_ATTRS);
      let order: number | undefined;
      if (attrs.order !== undefined) {
        if (!/^\d+(?:\.\d+)?$/.test(attrs.order)) {
          throw new DiagramInputError(`Invalid branch order: ${attrs.order.slice(0, 20)}`);
        }
        order = Number(attrs.order);
        if (!Number.isFinite(order) || order < 0) {
          throw new DiagramInputError(`Branch order must be non-negative: ${attrs.order}`);
        }
      }
      const curState = branchStates.get(currentBranch);
      const count = branchStates.size;
      checkCount(count + 1, MAX_NODES, "branches");
      const created: GitBranch = {
        name,
        order,
        createdIndex: count,
        colorIndex: count,
        ...(curState?.head !== undefined ? { parentCommitId: curState.head } : {}),
      };
      branchStates.set(name, { branch: created, head: curState?.head, headSeq: curState?.headSeq });
      currentBranch = name;
      continue;
    }

    if (cmd === "checkout" || cmd === "switch") {
      const { name, rest: trailing } = readBranchToken(rest, cmd);
      if (trailing !== "") {
        throw new DiagramInputError(
          `Unexpected trailing text after ${cmd}: ${trailing.slice(0, 30)}`,
        );
      }
      if (!branchStates.has(name)) {
        throw new DiagramInputError(`Unknown gitGraph branch: ${name}`);
      }
      currentBranch = name;
      continue;
    }

    if (cmd === "merge") {
      const { name: sourceName, rest: attrRest } = readBranchToken(rest, "merge");
      if (sourceName === currentBranch) {
        throw new DiagramInputError(`Cannot merge branch into itself: ${sourceName}`);
      }
      const sourceState = branchStates.get(sourceName);
      if (!sourceState) {
        throw new DiagramInputError(`Unknown branch to merge: ${sourceName}`);
      }
      if (sourceState.head === undefined || sourceState.headSeq === undefined) {
        throw new DiagramInputError(`Cannot merge branch without commits: ${sourceName}`);
      }
      const attrs = parseAttributes(attrRest, COMMIT_ATTRS);
      const curState = branchStates.get(currentBranch);
      if (!curState) throw new DiagramInputError("Missing current branch");
      const parents =
        curState.head !== undefined ? [curState.head, sourceState.head] : [sourceState.head];
      const parentSeqs =
        curState.headSeq !== undefined
          ? [curState.headSeq, sourceState.headSeq]
          : [sourceState.headSeq];
      totalEdges += parents.length;
      checkCount(totalEdges, MAX_EDGES, "edges");
      const seq = commits.length;
      const customId = attrs.id !== undefined;
      const id = attrs.id ?? `${String(curState.branch.colorIndex)}-${String(seq + 1)}`;
      if (customId && id === "") throw new DiagramInputError("Merge commit id cannot be empty");
      pushCommit({
        seq,
        id,
        customId,
        type: parseCommitType(attrs.type),
        ...(attrs.tag !== undefined ? { tag: attrs.tag } : {}),
        branch: currentBranch,
        parents,
        parentSeqs,
        isMerge: true,
      });
      continue;
    }

    // cherry-pick
    const attrs = parseAttributes(rest, CHERRY_ATTRS);
    if (!attrs.id) {
      throw new DiagramInputError("cherry-pick requires an id attribute");
    }
    const target = commitsById.get(attrs.id);
    if (!target) {
      throw new DiagramInputError(`Unknown cherry-pick commit id: ${attrs.id.slice(0, 30)}`);
    }
    if (target.branch === currentBranch) {
      throw new DiagramInputError("Cannot cherry-pick a commit from the current branch");
    }
    const curState = branchStates.get(currentBranch);
    if (!curState || curState.head === undefined || curState.headSeq === undefined) {
      throw new DiagramInputError("Current branch needs at least one commit before cherry-pick");
    }
    if (target.isMerge) {
      if (!attrs.parent) {
        throw new DiagramInputError("Cherry-picking a merge commit requires a parent attribute");
      }
      if (!target.parents.includes(attrs.parent)) {
        throw new DiagramInputError(`Invalid parent ${attrs.parent} for merge commit ${target.id}`);
      }
    } else if (attrs.parent !== undefined && !target.parents.includes(attrs.parent)) {
      throw new DiagramInputError(`Invalid parent ${attrs.parent} for commit ${target.id}`);
    }

    totalEdges += 2;
    checkCount(totalEdges, MAX_EDGES, "edges");
    const seq = commits.length;
    const id = `cp-${String(seq + 1)}`;
    pushCommit({
      seq,
      id,
      customId: false,
      type: "NORMAL",
      tag: attrs.tag ?? target.id,
      branch: currentBranch,
      parents: [curState.head],
      parentSeqs: [curState.headSeq],
      isMerge: false,
      cherryPickFrom: target.id,
      cherryPickFromSeq: target.seq,
    });
  }

  if (commits.length === 0) {
    throw new DiagramInputError("A gitGraph diagram needs at least one commit");
  }

  const branches = [...branchStates.values()]
    .map((s) => s.branch)
    .sort((a, b) => (a.order ?? 0) - (b.order ?? 0) || a.createdIndex - b.createdIndex);

  return { direction, branches, commits };
}
