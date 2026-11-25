/**
 * ZenUML diagram parser. Grammar follows Mermaid's zenuml page: title, explicit/implicit
 * participants with aliases and @Annotators, // comments (rendered above messages/fragments only),
 * sync calls with optional {} nesting and variable assignment, async -> messages, `new` creation,
 * `return` and @return/@reply, and loop/if-else/opt/par/try-catch-finally control blocks.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { clampLabel } from "../../measure";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** A participant or actor declared implicitly or explicitly in a ZenUML diagram. */
export interface ZenParticipant {
  id: string;
  label: string;
  annotator?: string;
  created: boolean;
}

/** One branch inside an `alt` (`if` / `else if` / `else`) block. */
export interface ZenBranch {
  label: string;
  body: ZenStatement[];
}

/** A parsed ZenUML statement. */
export type ZenStatement =
  | {
      kind: "sync";
      from?: string;
      to: string;
      method: string;
      assign?: string;
      comment?: string;
      body: ZenStatement[];
    }
  | {
      kind: "async";
      from: string;
      to: string;
      text: string;
      isReply: boolean;
      comment?: string;
    }
  | {
      kind: "new";
      from?: string;
      target: string;
      args: string;
      assign?: string;
      comment?: string;
      body: ZenStatement[];
    }
  | {
      kind: "return";
      from: string;
      to?: string;
      value: string;
      comment?: string;
    }
  | {
      kind: "loop";
      loopKind: "while" | "for" | "foreach" | "loop";
      condition: string;
      comment?: string;
      body: ZenStatement[];
    }
  | {
      kind: "alt";
      comment?: string;
      branches: ZenBranch[];
    }
  | {
      kind: "opt";
      label: string;
      comment?: string;
      body: ZenStatement[];
    }
  | {
      kind: "par";
      comment?: string;
      body: ZenStatement[];
    }
  | {
      kind: "try";
      comment?: string;
      tryBody: ZenStatement[];
      catchBody?: ZenStatement[];
      finallyBody?: ZenStatement[];
    };

/** A parsed ZenUML diagram. */
export interface ZenumlModel {
  title: string;
  participants: ZenParticipant[];
  statements: ZenStatement[];
}

const RESERVED = new Set([
  "title",
  "if",
  "else",
  "while",
  "for",
  "foreach",
  "loop",
  "opt",
  "par",
  "try",
  "catch",
  "finally",
  "return",
  "new",
]);
const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style|linkStyle)\b|:::/i;

type OpenFrame =
  | { kind: "sync"; stmt: Extract<ZenStatement, { kind: "sync" }>; pushedCaller: boolean }
  | { kind: "new"; stmt: Extract<ZenStatement, { kind: "new" }>; pushedCaller: boolean }
  | { kind: "loop"; stmt: Extract<ZenStatement, { kind: "loop" }> }
  | { kind: "alt"; stmt: Extract<ZenStatement, { kind: "alt" }> }
  | { kind: "opt"; stmt: Extract<ZenStatement, { kind: "opt" }> }
  | { kind: "par"; stmt: Extract<ZenStatement, { kind: "par" }> }
  | {
      kind: "try";
      stmt: Extract<ZenStatement, { kind: "try" }>;
      phase: "try" | "catch" | "finally";
    };

function unquote(raw: string): string {
  const t = raw.trim();
  if (t.startsWith('"') && t.endsWith('"') && t.length >= 2) return t.slice(1, -1);
  return t;
}

/** Parse `zenuml` source (text or prepared lines); throws DiagramInputError on syntax/brace errors. */
export function parseZenuml(input: string | PreparedSource): ZenumlModel {
  const [, ...body] = linesOf(input, "zenuml");
  const model: ZenumlModel = {
    title: "",
    participants: [],
    statements: [],
  };

  const byId = new Map<string, ZenParticipant>();
  const frames: OpenFrame[] = [];
  const callerStack: string[] = [];
  let pendingComments: string[] = [];
  let pendingReply = false;
  let stmtCount = 0;
  let lastClosedAlt: Extract<ZenStatement, { kind: "alt" }> | undefined;
  let lastClosedTry: Extract<ZenStatement, { kind: "try" }> | undefined;

  const ensureParticipant = (
    id: string,
    opts?: { label?: string; annotator?: string; created?: boolean },
  ): ZenParticipant => {
    let found = byId.get(id);
    if (!found) {
      found = {
        id,
        label: clampLabel(opts?.label ?? id),
        created: opts?.created ?? false,
      };
      if (opts?.annotator !== undefined) found.annotator = opts.annotator;
      byId.set(id, found);
      model.participants.push(found);
      checkCount(model.participants.length, MAX_NODES, "participants");
    } else {
      if (opts?.label !== undefined) found.label = clampLabel(opts.label);
      if (opts?.annotator !== undefined) found.annotator = opts.annotator;
      if (opts?.created) found.created = true;
    }
    return found;
  };

  const currentBody = (): ZenStatement[] => {
    const top = frames[frames.length - 1];
    if (!top) return model.statements;
    switch (top.kind) {
      case "sync":
      case "new":
      case "loop":
      case "opt":
      case "par":
        return top.stmt.body;
      case "alt": {
        const br = top.stmt.branches[top.stmt.branches.length - 1];
        if (!br) throw new DiagramInputError("Alt block missing branch");
        return br.body;
      }
      case "try":
        if (top.phase === "catch") return (top.stmt.catchBody ??= []);
        if (top.phase === "finally") return (top.stmt.finallyBody ??= []);
        return top.stmt.tryBody;
    }
  };

  const pushStmt = (stmt: ZenStatement): void => {
    stmtCount++;
    checkCount(stmtCount, MAX_EDGES, "statements");
    currentBody().push(stmt);
  };

  const takeComment = (): string | undefined => {
    if (pendingComments.length === 0) return undefined;
    const joined = clampLabel(pendingComments.join("\n"));
    pendingComments = [];
    return joined;
  };

  const openDepth = (): void => {
    if (frames.length >= MAX_DEPTH) {
      throw new DiagramLimitError(
        `Diagram too large: blocks nested more than ${String(MAX_DEPTH)} deep`,
      );
    }
  };

  const closeTopFrame = (): void => {
    const popped = frames.pop();
    if (!popped) throw new DiagramInputError("Unexpected `}` without matching `{`");
    if ((popped.kind === "sync" || popped.kind === "new") && popped.pushedCaller) {
      callerStack.pop();
    }
    lastClosedAlt = popped.kind === "alt" ? popped.stmt : undefined;
    lastClosedTry = popped.kind === "try" ? popped.stmt : undefined;
  };

  for (const raw of body) {
    let line = raw.trim();
    if (line === "") continue;

    if (line.startsWith("//")) {
      pendingComments.push(line.slice(2).trim());
      continue;
    }

    if (REFUSED.test(line)) {
      throw new DiagramInputError(`Not drawn: ${line.split(/\s/)[0] ?? ""}`);
    }

    // Handle leading `}` on `} else ...` / `} catch` / `} finally` lines.
    let closedInline = false;
    if (line.startsWith("}")) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      pendingComments = [];
      closeTopFrame();
      closedInline = true;
      line = line.slice(1).trim();
      if (line === "") continue;
    }

    // Check `else if` / `else` continuation right after closing an `alt` frame.
    const elseIfMatch = /^else\s+if\s*\(([^)]*)\)\s*\{$/i.exec(line);
    const elseMatch = /^else\s*\{$/i.exec(line);
    if (elseIfMatch || elseMatch) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      pendingComments = [];
      if (!lastClosedAlt) {
        throw new DiagramInputError("`else` without preceding `if` block");
      }
      const altStmt = lastClosedAlt;
      lastClosedAlt = undefined;
      lastClosedTry = undefined;
      altStmt.branches.push({
        label: elseIfMatch ? clampLabel((elseIfMatch[1] ?? "").trim()) : "else",
        body: [],
      });
      frames.push({ kind: "alt", stmt: altStmt });
      continue;
    }

    // Check `catch` / `finally` continuation right after closing a `try` frame.
    const catchMatch = /^catch(?:\s*\([^)]*\))?\s*\{$/i.exec(line);
    const finallyMatch = /^finally\s*\{$/i.exec(line);
    if (catchMatch || finallyMatch) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      pendingComments = [];
      if (!lastClosedTry) {
        throw new DiagramInputError(
          `${catchMatch ? "catch" : "finally"} without preceding \`try\` block`,
        );
      }
      const tryStmt = lastClosedTry;
      lastClosedAlt = undefined;
      lastClosedTry = undefined;
      const phase = catchMatch ? "catch" : "finally";
      if (phase === "catch") tryStmt.catchBody ??= [];
      else tryStmt.finallyBody ??= [];
      frames.push({ kind: "try", stmt: tryStmt, phase });
      continue;
    }

    if (closedInline) {
      throw new DiagramInputError(`Unexpected tokens after \`}\`: ${line.slice(0, 40)}`);
    }

    lastClosedAlt = undefined;
    lastClosedTry = undefined;

    if (/^@(?:return|reply)$/i.test(line)) {
      pendingReply = true;
      continue;
    }

    const titleMatch = /^title\s*:?\s*(.+)$/i.exec(line);
    if (titleMatch && frames.length === 0) {
      pendingComments = [];
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      model.title = clampLabel((titleMatch[1] ?? "").trim());
      continue;
    }

    // Control blocks: loop, if, opt, par, try.
    // Remove the required brace before matching conditions, so whitespace runs cannot trade
    // characters with a lazy condition while searching for a missing terminator.
    const blockHeader = line.endsWith("{") ? line.slice(0, -1).trimEnd() : undefined;
    const loopMatch =
      blockHeader === undefined
        ? null
        : /^(while|foreach|for|loop)(?:\s*\(([^)]*)\)|\s([^{]+))?$/i.exec(blockHeader);
    if (loopMatch) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      openDepth();
      const rawKind = (loopMatch[1] ?? "loop").toLowerCase();
      const loopKind = (rawKind === "foreach" ? "foreach" : rawKind) as
        "while" | "for" | "foreach" | "loop";
      const condition = clampLabel((loopMatch[2] ?? loopMatch[3] ?? "").trim());
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "loop" }> = {
        kind: "loop",
        loopKind,
        condition,
        body: [],
      };
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      frames.push({ kind: "loop", stmt });
      continue;
    }

    const ifMatch = /^if\s*\(([^)]*)\)\s*\{$/i.exec(line);
    if (ifMatch) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      openDepth();
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "alt" }> = {
        kind: "alt",
        branches: [{ label: clampLabel((ifMatch[1] ?? "").trim()), body: [] }],
      };
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      frames.push({ kind: "alt", stmt });
      continue;
    }

    const optMatch =
      blockHeader === undefined ? null : /^opt(?:\s*\(([^)]*)\)|\s([^{]+))?$/i.exec(blockHeader);
    if (optMatch) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      openDepth();
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "opt" }> = {
        kind: "opt",
        label: clampLabel((optMatch[1] ?? optMatch[2] ?? "").trim()),
        body: [],
      };
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      frames.push({ kind: "opt", stmt });
      continue;
    }

    if (/^par\s*\{$/i.test(line)) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      openDepth();
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "par" }> = { kind: "par", body: [] };
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      frames.push({ kind: "par", stmt });
      continue;
    }

    if (/^try\s*\{$/i.test(line)) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      openDepth();
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "try" }> = { kind: "try", tryBody: [] };
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      frames.push({ kind: "try", stmt, phase: "try" });
      continue;
    }

    // `return expr` inside a sync/new block.
    const returnMatch = /^return(?:\s+(.+))?$/i.exec(line);
    if (returnMatch) {
      if (pendingReply) throw new DiagramInputError("@return must precede an async message");
      const activeCallee = callerStack[callerStack.length - 1];
      if (!activeCallee) {
        throw new DiagramInputError("`return` must appear inside a sync or creation block");
      }
      const parentCaller = callerStack[callerStack.length - 2];
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "return" }> = {
        kind: "return",
        from: activeCallee,
        value: clampLabel((returnMatch[1] ?? "").trim()),
      };
      if (parentCaller !== undefined) stmt.to = parentCaller;
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      continue;
    }

    // Async message: `Alice->Bob: text` (check that right side is NOT a dot sync call!).
    const asyncMatch = /^([A-Za-z_][A-Za-z0-9_-]*)\s*->\s*([A-Za-z_][A-Za-z0-9_-]*)\s*:(.*)$/.exec(
      line,
    );
    if (asyncMatch) {
      const from = ensureParticipant(asyncMatch[1] ?? "").id;
      const to = ensureParticipant(asyncMatch[2] ?? "").id;
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "async" }> = {
        kind: "async",
        from,
        to,
        text: clampLabel((asyncMatch[3] ?? "").trim()),
        isReply: pendingReply,
      };
      if (comment !== undefined) stmt.comment = comment;
      pendingReply = false;
      pushStmt(stmt);
      continue;
    }

    if (pendingReply) {
      throw new DiagramInputError("@return must precede an async `A->B: text` message");
    }

    // Creation message: `[var = ]new Target[(args)][ {]`
    const newMatch =
      /^(?:([A-Za-z_](?:[A-Za-z0-9_<>\s]*[A-Za-z0-9_<>])?)\s*=\s*)?new\s+([A-Za-z_][A-Za-z0-9_-]*)(?:\(([^)]*)\))?(?:\s*(\{))?$/i.exec(
        line,
      );
    if (newMatch) {
      const hasBrace = newMatch[4] === "{";
      if (hasBrace) openDepth();
      const target = ensureParticipant(newMatch[2] ?? "", { created: true }).id;
      const from = callerStack[callerStack.length - 1];
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "new" }> = {
        kind: "new",
        target,
        args: clampLabel((newMatch[3] ?? "").trim()),
        body: [],
      };
      if (from !== undefined) stmt.from = from;
      if (newMatch[1]?.trim()) stmt.assign = clampLabel(newMatch[1].trim());
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      if (hasBrace) {
        callerStack.push(target);
        frames.push({ kind: "new", stmt, pushedCaller: true });
      }
      continue;
    }

    // Sync message: `[assign = ][Sender->]Target.method[(args)][ {]`
    const syncMatch =
      /^(?:([A-Za-z_](?:[A-Za-z0-9_<>\s]*[A-Za-z0-9_<>])?)\s*=\s*)?(?:([A-Za-z_][A-Za-z0-9_-]*)\s*->\s*)?([A-Za-z_][A-Za-z0-9_-]*)\.([A-Za-z_][A-Za-z0-9_]*(?:\([^)]*\))?)(?:\s*(\{))?$/.exec(
        line,
      );
    if (syncMatch) {
      const hasBrace = syncMatch[5] === "{";
      if (hasBrace) openDepth();
      const explicitFrom = syncMatch[2] ? ensureParticipant(syncMatch[2]).id : undefined;
      const to = ensureParticipant(syncMatch[3] ?? "").id;
      const from = explicitFrom ?? callerStack[callerStack.length - 1];
      const comment = takeComment();
      const stmt: Extract<ZenStatement, { kind: "sync" }> = {
        kind: "sync",
        to,
        method: clampLabel(syncMatch[4] ?? ""),
        body: [],
      };
      if (from !== undefined) stmt.from = from;
      if (syncMatch[1]?.trim()) stmt.assign = clampLabel(syncMatch[1].trim());
      if (comment !== undefined) stmt.comment = comment;
      pushStmt(stmt);
      if (hasBrace) {
        callerStack.push(to);
        frames.push({ kind: "sync", stmt, pushedCaller: true });
      }
      continue;
    }

    // Explicit participant declaration at top level: `[@Annotator ]id[ as Label]`.
    const partMatch =
      /^(?:@([A-Za-z][A-Za-z0-9_-]*)\s+)?([A-Za-z_][A-Za-z0-9_-]*)(?:\s+as\s+(.+))?$/.exec(line);
    if (partMatch && frames.length === 0) {
      const id = partMatch[2] ?? "";
      if (!RESERVED.has(id.toLowerCase())) {
        pendingComments = [];
        const opts: { label?: string; annotator?: string } = {};
        if (partMatch[1] !== undefined) opts.annotator = partMatch[1].toLowerCase();
        if (partMatch[3] !== undefined) opts.label = unquote(partMatch[3]);
        ensureParticipant(id, opts);
        continue;
      }
    }

    throw new DiagramInputError(`Unreadable zenuml line: ${line.slice(0, 40)}`);
  }

  if (pendingReply) throw new DiagramInputError("@return without a following message");
  if (frames.length > 0) throw new DiagramInputError("Unclosed `{` block in zenuml");
  if (model.participants.length === 0 || model.statements.length === 0) {
    throw new DiagramInputError("A ZenUML diagram needs participants and statements");
  }
  return model;
}
