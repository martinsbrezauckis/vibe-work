/**
 * Agentflow diagram parser. Grammar follows Mermaid's agentflow page: `agentflow[-beta] [dir]`,
 * nested `flow ... end` containers, `global ... end` top-level blocks, `connector` nodes,
 * domain shape aliases (`task`, `tool`, `input`, `decision`, `refdoc`, `action`), single/multiline
 * YAML-like `@{ ... }` metadata (stripping prototype keys), `view: collapsed`, and `-->`, `-.-`,
 * `--x` edges with labels and chaining.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** Layout direction of an agentflow diagram. */
export type AgentflowDirection = "TB" | "BT" | "LR" | "RL";
/** Semantic edge kind in agentflow. */
export type AgentflowEdgeKind = "sequence" | "reference" | "failure";

/** A `flow ... end` container (which may nest and/or collapse). */
export interface AgentContainer {
  id: string;
  label: string;
  parentId?: string;
  collapsed: boolean;
  metadata: Record<string, string>;
}

/** A task, tool, input, decision, refdoc, action, or connector node. */
export interface AgentNode {
  id: string;
  label: string;
  shape: string;
  containerId?: string;
  isGlobal: boolean;
  isConnector: boolean;
  metadata: Record<string, string>;
  explicitScope: boolean;
}

/** A directed semantic edge between two nodes or containers. */
export interface AgentEdge {
  from: string;
  to: string;
  kind: AgentflowEdgeKind;
  label: string;
  metadata: Record<string, string>;
}

/** A parsed agentflow diagram. */
export interface AgentflowModel {
  direction: AgentflowDirection;
  containers: Map<string, AgentContainer>;
  nodes: Map<string, AgentNode>;
  connectors: Map<string, AgentNode>;
  edges: AgentEdge[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;
const PROTO_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function unquote(raw: string): string {
  const t = raw.trim();
  if (
    ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'"))) &&
    t.length >= 2
  ) {
    return t.slice(1, -1);
  }
  return t;
}

function mergeMetadataLines(body: readonly string[]): string[] {
  const out: string[] = [];
  let i = 0;
  while (i < body.length) {
    let cur = (body[i] ?? "").trim();
    i++;
    if (!cur) continue;
    const at = cur.indexOf("@{");
    if (at >= 0) {
      let depth = 0;
      let quote: '"' | "'" | undefined;
      const scan = (s: string): void => {
        for (let k = 0; k < s.length; k++) {
          const ch = s[k] ?? "";
          if (quote) {
            if (ch === "\\" && k + 1 < s.length) k++;
            else if (ch === quote) quote = undefined;
          } else if (ch === '"' || ch === "'") quote = ch;
          else if (ch === "{") depth++;
          else if (ch === "}") depth--;
        }
      };
      scan(cur.slice(at + 1));
      while (depth > 0 && i < body.length) {
        const next = (body[i] ?? "").trim();
        i++;
        cur += `\n${next}`;
        scan(next);
      }
      if (depth > 0) throw new DiagramInputError("Unterminated `@{ ... }` metadata block");
    }
    out.push(cur);
  }
  return out;
}

function parseYamlMeta(raw: string): Record<string, string> {
  const safe: Record<string, string> = {};
  const parts = raw
    .split(/\n|,(?=(?:[^"']*["'][^"']*["'])*[^"']*$)/)
    .map((p) => p.trim())
    .filter(Boolean);
  for (const part of parts) {
    const colon = part.indexOf(":");
    if (colon <= 0) throw new DiagramInputError(`Invalid metadata entry: ${part}`);
    const key = part.slice(0, colon).trim();
    if (PROTO_KEYS.has(key)) continue;
    const val = clampLabel(unquote(part.slice(colon + 1).trim()));
    Object.defineProperty(safe, key, {
      value: val,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return safe;
}

function mergeMeta(target: Record<string, string>, incoming: Record<string, string>): void {
  for (const [k, v] of Object.entries(incoming)) {
    if (PROTO_KEYS.has(k)) continue;
    Object.defineProperty(target, k, {
      value: v,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
}

function parseNodePart(raw: string): { id: string; label?: string; meta?: Record<string, string> } {
  let rest = raw.trim();
  let meta: Record<string, string> | undefined;
  const atIdx = rest.indexOf("@{");
  if (atIdx >= 0) {
    if (!rest.endsWith("}")) throw new DiagramInputError("Unterminated `@{ ... }` on node");
    meta = parseYamlMeta(rest.slice(atIdx + 2, -1));
    rest = rest.slice(0, atIdx).trim();
  }
  const bracketMatch = /^([A-Za-z0-9_.-]+)\s*(?:\[([\s\S]+)\]|\(([\s\S]+)\))$/.exec(rest);
  if (bracketMatch?.[1]) {
    const label = clampLabel(unquote((bracketMatch[2] ?? bracketMatch[3] ?? "").trim()));
    const res: { id: string; label?: string; meta?: Record<string, string> } = {
      id: bracketMatch[1],
      label,
    };
    if (meta !== undefined) res.meta = meta;
    return res;
  }
  if (/^[A-Za-z0-9_.-]+$/.test(rest)) {
    const res: { id: string; label?: string; meta?: Record<string, string> } = { id: rest };
    if (meta !== undefined) res.meta = meta;
    return res;
  }
  throw new DiagramInputError(`Invalid agentflow node: ${raw.slice(0, 40)}`);
}

interface ParsedOp {
  index: number;
  length: number;
  kind: AgentflowEdgeKind;
  label: string;
}

function findNextEdgeOp(text: string, fromIdx: number): ParsedOp | undefined {
  let sq = 0;
  let br = 0;
  let quote: '"' | "'" | undefined;
  for (let i = fromIdx; i < text.length; i++) {
    const ch = text[i] ?? "";
    if (quote) {
      if (ch === "\\" && i + 1 < text.length) i++;
      else if (ch === quote) quote = undefined;
      continue;
    }
    if (ch === '"' || ch === "'") quote = ch;
    else if (ch === "[") sq++;
    else if (ch === "]" && sq > 0) sq--;
    else if (ch === "{") br++;
    else if (ch === "}" && br > 0) br--;
    if (sq > 0 || br > 0) continue;

    if (ch !== "-") continue;
    // The first disallowed character after a label is the start of its closing token.
    // Scanning once avoids partitioning long whitespace runs between regex quantifiers.
    const dotted = text.startsWith("-.", i);
    const plain = !dotted && text.startsWith("--", i);
    if (dotted || plain) {
      let cursor = i + 2;
      if (/\s/.test(text[cursor] ?? "")) {
        cursor++;
        const labelStart = cursor;
        while (
          cursor < text.length &&
          (dotted ? text[cursor] !== "." : !/[->]/.test(text[cursor] ?? ""))
        )
          cursor++;
        const label = text.slice(labelStart, cursor);
        const endTok = dotted
          ? text.startsWith(".-", cursor)
            ? ".-"
            : undefined
          : text.startsWith("-->", cursor)
            ? "-->"
            : text.startsWith("--x", cursor)
              ? "--x"
              : undefined;
        if (endTok && label.length >= 2 && /\s$/.test(label)) {
          return {
            index: i,
            length: cursor + endTok.length - i,
            kind: endTok === "--x" ? "failure" : endTok === ".-" ? "reference" : "sequence",
            label: clampLabel(unquote(label.trim())),
          };
        }
      }
    }
    const slice = text.slice(i);
    const op = /^(-->|-\.-|--x)(?:\|([^|]*)\|)?/.exec(slice);
    if (op?.[1]) {
      return {
        index: i,
        length: op[0].length,
        kind: op[1] === "--x" ? "failure" : op[1] === "-.-" ? "reference" : "sequence",
        label: clampLabel(unquote(op[2] ?? "")),
      };
    }
  }
  return undefined;
}

/** Parse `agentflow[-beta]` source (text or prepared lines); throws DiagramInputError on invalid syntax. */
export function parseAgentflow(input: string | PreparedSource): AgentflowModel {
  const [header, ...rawBody] = linesOf(input, "agentflow");
  const headerMatch = /^agentflow(?:-beta)?(?:\s+(TB|TD|BT|LR|RL))?\s*$/i.exec(
    (header ?? "").trim(),
  );
  if (!headerMatch) {
    throw new DiagramInputError(`Invalid agentflow header: ${(header ?? "").slice(0, 40)}`);
  }
  const rawDir = (headerMatch[1] ?? "TB").toUpperCase();
  const direction = (rawDir === "TD" ? "TB" : rawDir) as AgentflowDirection;

  const containers = new Map<string, AgentContainer>();
  const nodes = new Map<string, AgentNode>();
  const connectors = new Map<string, AgentNode>();
  const edges: AgentEdge[] = [];
  const stack: ({ kind: "flow"; id: string } | { kind: "global" })[] = [];
  const lines = mergeMetadataLines(rawBody);

  const currentFlowId = (): string | undefined => {
    for (let i = stack.length - 1; i >= 0; i--) {
      const entry = stack[i];
      if (entry?.kind === "global") return undefined;
      if (entry?.kind === "flow") return entry.id;
    }
    return undefined;
  };
  const inGlobal = (): boolean => stack.some((s) => s.kind === "global");

  const checkTotalItems = (): void => {
    checkCount(nodes.size + containers.size + connectors.size, MAX_NODES, "agentflow items");
  };

  const upsertNode = (
    raw: string,
    opts?: { isConnector?: boolean; explicitDecl?: boolean },
  ): string => {
    const parsed = parseNodePart(raw);
    if (!opts?.isConnector && containers.has(parsed.id) && !parsed.label && !parsed.meta) {
      return parsed.id;
    }
    let existing =
      (opts?.isConnector ? connectors.get(parsed.id) : nodes.get(parsed.id)) ?? undefined;
    if (!existing) {
      const scopeFlow = opts?.isConnector ? undefined : currentFlowId();
      existing = {
        id: parsed.id,
        label: parsed.label ?? parsed.id,
        shape: parsed.meta?.shape ?? (opts?.isConnector ? "connector" : "task"),
        isGlobal: inGlobal(),
        isConnector: opts?.isConnector ?? false,
        metadata: {},
        explicitScope: opts?.explicitDecl ?? false,
      };
      if (scopeFlow !== undefined && !existing.isGlobal) existing.containerId = scopeFlow;
      if (opts?.isConnector) connectors.set(existing.id, existing);
      else nodes.set(existing.id, existing);
      checkTotalItems();
    } else if (opts?.explicitDecl) {
      const scopeFlow = currentFlowId();
      existing.explicitScope = true;
      if (inGlobal()) {
        existing.isGlobal = true;
        delete existing.containerId;
      } else if (scopeFlow !== undefined) {
        existing.containerId = scopeFlow;
      } else {
        delete existing.containerId;
      }
    }
    if (parsed.label !== undefined) existing.label = parsed.label;
    if (parsed.meta) {
      mergeMeta(existing.metadata, parsed.meta);
      if (parsed.meta.shape) existing.shape = parsed.meta.shape;
      if (parsed.meta.label) existing.label = parsed.meta.label;
    }
    return existing.id;
  };

  for (const line of lines) {
    if (/^(?:accTitle|accDescr)\b/i.test(line)) continue;
    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    if (/^global\b/i.test(line)) {
      if (!/^global$/i.test(line)) {
        throw new DiagramInputError("`global` takes no identifier, label, or metadata");
      }
      if (stack.length >= MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: flows nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      stack.push({ kind: "global" });
      continue;
    }

    if (/^end$/i.test(line)) {
      if (stack.length === 0)
        throw new DiagramInputError("`end` without matching `flow` or `global`");
      stack.pop();
      continue;
    }

    const flowMatch = /^flow\s+(.+)$/i.exec(line);
    if (flowMatch) {
      if (stack.length >= MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: flows nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      const parsed = parseNodePart(flowMatch[1] ?? "");
      if (containers.has(parsed.id)) {
        throw new DiagramInputError(`Duplicate flow container: ${parsed.id}`);
      }
      const parentId = currentFlowId();
      const container: AgentContainer = {
        id: parsed.id,
        label: parsed.label ?? parsed.id,
        collapsed: parsed.meta?.view === "collapsed",
        metadata: {},
      };
      if (parentId !== undefined) container.parentId = parentId;
      if (parsed.meta) mergeMeta(container.metadata, parsed.meta);
      containers.set(container.id, container);
      checkTotalItems();
      stack.push({ kind: "flow", id: container.id });
      continue;
    }

    const connMatch = /^connector\s+(.+)$/i.exec(line);
    if (connMatch) {
      upsertNode(connMatch[1] ?? "", { isConnector: true, explicitDecl: true });
      continue;
    }

    // Check standalone metadata attachment `id@{ ... }` on container, connector, or node.
    const metaOnly = /^([A-Za-z0-9_.-]+)@\{([\s\S]*)\}$/.exec(line);
    if (metaOnly?.[1]) {
      const id = metaOnly[1];
      const meta = parseYamlMeta(metaOnly[2] ?? "");
      const cont = containers.get(id);
      if (cont) {
        mergeMeta(cont.metadata, meta);
        if (meta.view === "collapsed") cont.collapsed = true;
        if (meta.label) cont.label = meta.label;
        continue;
      }
      const conn = connectors.get(id);
      if (conn) {
        mergeMeta(conn.metadata, meta);
        if (meta.label) conn.label = meta.label;
        continue;
      }
      upsertNode(line, { explicitDecl: false });
      continue;
    }

    // Check edges / chains.
    const firstOp = findNextEdgeOp(line, 0);
    if (firstOp) {
      const endpoints: string[] = [];
      const ops: Omit<ParsedOp, "index" | "length">[] = [];
      let cursor = 0;
      let op: ParsedOp | undefined = firstOp;
      while (op) {
        const lhs = line.slice(cursor, op.index).trim();
        if (!lhs) throw new DiagramInputError(`Missing source before edge: ${line.slice(0, 40)}`);
        endpoints.push(upsertNode(lhs, { explicitDecl: false }));
        ops.push({ kind: op.kind, label: op.label });
        cursor = op.index + op.length;
        op = findNextEdgeOp(line, cursor);
      }
      const tail = line.slice(cursor).trim();
      if (!tail) throw new DiagramInputError(`Dangling edge without target: ${line.slice(0, 40)}`);
      endpoints.push(upsertNode(tail, { explicitDecl: false }));
      ops.forEach((edgeOp, idx) => {
        edges.push({
          from: endpoints[idx] ?? "",
          to: endpoints[idx + 1] ?? "",
          kind: edgeOp.kind,
          label: edgeOp.label,
          metadata: {},
        });
        checkCount(edges.length, MAX_EDGES, "edges");
      });
      continue;
    }

    upsertNode(line, { explicitDecl: true });
  }

  if (stack.length > 0) throw new DiagramInputError("Unclosed `flow` or `global` block");
  if (nodes.size === 0 && containers.size === 0 && connectors.size === 0) {
    throw new DiagramInputError("An agentflow diagram needs nodes or containers");
  }
  return { direction, containers, nodes, connectors, edges };
}
