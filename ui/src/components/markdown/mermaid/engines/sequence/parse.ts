/**
 * Sequence diagram parser. Grammar follows Mermaid's sequence diagram page: participants and
 * actors with aliases and boxes, create/destroy, ten arrow kinds with inline activation, notes,
 * activate/deactivate, autonumber, and nested loop/alt/opt/par/critical/break/rect blocks. `link`,
 * `links` and `properties` lines are read and dropped: this renderer never makes anything clickable.
 */
import { linesOf } from "../../detect";
import { MAX_DEPTH, MAX_EDGES, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, DiagramLimitError, type PreparedSource } from "../../types";

/** The ten message arrow styles: line style (solid or dotted) crossed with the end (none, arrowhead, cross, open, both). */
export type ArrowKind =
  | "solid"
  | "dotted"
  | "solid-arrow"
  | "dotted-arrow"
  | "solid-cross"
  | "dotted-cross"
  | "solid-async"
  | "dotted-async"
  | "solid-both"
  | "dotted-both";

/** A participant or actor with its display label and box, creation and destruction flags. */
export interface SequenceParticipant {
  id: string;
  label: string;
  kind: "participant" | "actor";
  /** Index into `boxes`, when declared inside a box. */
  box?: number;
  /** Declared by `create`: its header is drawn where it is created. */
  created: boolean;
  destroyed: boolean;
}
/** A named group of participants declared in one `box`. */
export interface SequenceBox {
  label: string;
  participants: string[];
}
/** Control blocks: loop, alt, opt, par, critical, break and rect. */
export type BlockKind = "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect";
/** One section of a block (the main part, an `else`, `and` or `option` part) with its statements. */
export interface SequenceBlockSection {
  /** Text after loop/else/and/option; the first section carries the block's own label. */
  label: string;
  statements: SequenceStatement[];
}
/** One statement: message, note, activation change or nested block. */
export type SequenceStatement =
  | {
      kind: "message";
      from: string;
      to: string;
      text: string;
      arrow: ArrowKind;
      /** `+` activates the receiver, `-` deactivates the sender. */
      activation?: "+" | "-" | undefined;
      created?: boolean;
      /** Id of the participant destroyed by this message. */
      destroyed?: string;
    }
  | {
      kind: "note";
      placement: "left" | "right" | "over";
      participants: string[];
      text: string;
    }
  | { kind: "activate"; id: string }
  | { kind: "deactivate"; id: string }
  | {
      kind: "block";
      block: BlockKind;
      sections: SequenceBlockSection[];
    };
/** A parsed sequence diagram. */
export interface SequenceModel {
  title: string;
  autonumber: { start: number; step: number } | undefined;
  participants: SequenceParticipant[];
  boxes: SequenceBox[];
  statements: SequenceStatement[];
}

const ARROWS: [string, ArrowKind][] = [
  ["<<-->>", "dotted-both"],
  ["<<->>", "solid-both"],
  ["-->>", "dotted-arrow"],
  ["->>", "solid-arrow"],
  ["--x", "dotted-cross"],
  ["-x", "solid-cross"],
  ["--)", "dotted-async"],
  ["-)", "solid-async"],
  ["-->", "dotted"],
  ["->", "solid"],
];
const MESSAGE = new RegExp(
  `^(.+?)\\s*(${ARROWS.map(([token]) => token.replace(/[()|]/g, "\\$&")).join("|")})\\s*([+-])?\\s*(.+?)\\s*:(.*)$`,
);
const BLOCK_OPEN = /^(loop|alt|opt|par|critical|break|rect)(?:\s+(.*))?$/i;
const SECTION_WORD: Partial<Record<BlockKind, RegExp>> = {
  alt: /^else(?:\s+(.*))?$/i,
  par: /^and(?:\s+(.*))?$/i,
  critical: /^option(?:\s+(.*))?$/i,
};
const BOX_COLOUR = /^(?:rgba?\([^)]*\)|#[0-9a-f]{3,8}|transparent)\s*/i;

interface Frame {
  block: BlockKind;
  sections: SequenceBlockSection[];
}

function unquote(raw: string): string {
  const text = raw.trim();
  if (text.startsWith('"')) {
    if (text.length < 2 || !text.endsWith('"')) throw new DiagramInputError("Unterminated quote");
    return text.slice(1, -1);
  }
  return text;
}

/** Parse `sequenceDiagram` source (text or prepared lines); throws DiagramInputError on anything unreadable or unclosed. */
export function parseSequence(input: string | PreparedSource): SequenceModel {
  const [, ...body] = linesOf(input, "sequence");
  const model: SequenceModel = {
    title: "",
    autonumber: undefined,
    participants: [],
    boxes: [],
    statements: [],
  };
  const byId = new Map<string, SequenceParticipant>();
  const frames: Frame[] = [];
  let openBox: number | undefined;
  let statementCount = 0;
  let messageCount = 0;
  let pendingCreate: string | undefined;
  let pendingDestroy: string | undefined;

  const target = (): SequenceStatement[] => {
    const frame = frames[frames.length - 1];
    if (!frame) return model.statements;
    const section = frame.sections[frame.sections.length - 1];
    if (!section) throw new DiagramInputError("Block without a section");
    return section.statements;
  };
  const declare = (
    id: string,
    label: string | undefined,
    kind: "participant" | "actor",
    explicit: boolean,
  ): SequenceParticipant => {
    let found = byId.get(id);
    if (!found) {
      found = {
        id,
        label: label ?? id,
        kind,
        created: false,
        destroyed: false,
      };
      byId.set(id, found);
      model.participants.push(found);
      checkCount(model.participants.length, MAX_NODES, "participants");
    } else if (explicit) {
      if (label !== undefined) found.label = label;
      found.kind = kind;
    }
    if (explicit && openBox !== undefined && found.box === undefined) {
      found.box = openBox;
      model.boxes[openBox]?.participants.push(id);
    }
    return found;
  };
  const known = (id: string): string => {
    declare(id, undefined, "participant", false);
    return id;
  };
  const push = (statement: SequenceStatement): void => {
    statementCount++;
    checkCount(statementCount, MAX_EDGES * 2, "statements");
    target().push(statement);
  };

  for (const raw of body) {
    const line = raw.trim().replace(/;$/, "").trim();
    if (line === "") continue;
    let match: RegExpExecArray | null;
    if (openBox !== undefined) {
      if (/^end$/i.test(line)) {
        openBox = undefined;
        continue;
      }
      match = /^(participant|actor)\s+(\S+)(?:\s+as\s+(.+))?$/i.exec(line);
      if (!match) throw new DiagramInputError("Only participants belong in a box");
    }
    if ((match = /^(participant|actor)\s+(\S+)(?:\s+as\s+(.+))?$/i.exec(line))) {
      const kind = (match[1] ?? "").toLowerCase() as "participant" | "actor";
      declare(match[2] ?? "", match[3] === undefined ? undefined : unquote(match[3]), kind, true);
    } else if ((match = /^create\s+(participant|actor)\s+(\S+)(?:\s+as\s+(.+))?$/i.exec(line))) {
      const kind = (match[1] ?? "").toLowerCase() as "participant" | "actor";
      const id = match[2] ?? "";
      if (byId.has(id)) throw new DiagramInputError("Created participant already exists");
      declare(id, match[3] === undefined ? undefined : unquote(match[3]), kind, true).created =
        true;
      pendingCreate = id;
    } else if ((match = /^destroy\s+(\S+)$/i.exec(line))) {
      pendingDestroy = known(match[1] ?? "");
    } else if ((match = /^box(?:\s+(.*))?$/i.exec(line))) {
      if (frames.length > 0) throw new DiagramInputError("A box cannot sit inside a block");
      const label = (match[1] ?? "").trim().replace(BOX_COLOUR, "").trim();
      model.boxes.push({ label, participants: [] });
      openBox = model.boxes.length - 1;
    } else if ((match = /^title\s*:?\s*(.+)$/i.exec(line))) {
      model.title = (match[1] ?? "").trim();
    } else if ((match = /^autonumber(?:\s+(off|\d+)(?:\s+(\d+))?)?$/i.exec(line))) {
      model.autonumber =
        match[1]?.toLowerCase() === "off"
          ? undefined
          : {
              start: match[1] ? Number(match[1]) : 1,
              step: match[2] ? Number(match[2]) : 1,
            };
    } else if (/^(?:link|links|properties)\s/i.test(line) || /^acc(?:Title|Descr)\b/i.test(line)) {
      continue;
    } else if ((match = /^(activate|deactivate)\s+(\S+)$/i.exec(line))) {
      const id = known(match[2] ?? "");
      push(
        (match[1] ?? "").toLowerCase() === "activate"
          ? { kind: "activate", id }
          : { kind: "deactivate", id },
      );
    } else if ((match = /^note\s+(left of|right of|over)\s+([^:]+?)\s*:(.*)$/i.exec(line))) {
      const where = (match[1] ?? "").toLowerCase();
      const ids = (match[2] ?? "")
        .split(",")
        .map((part) => part.trim())
        .filter(Boolean);
      if (ids.length === 0 || (where !== "over" && ids.length !== 1) || ids.length > 2) {
        throw new DiagramInputError("A note names one participant, or two with `over`");
      }
      push({
        kind: "note",
        placement: where === "over" ? "over" : where.startsWith("left") ? "left" : "right",
        participants: ids.map(known),
        text: (match[3] ?? "").trim(),
      });
    } else if (/^end$/i.test(line)) {
      const frame = frames.pop();
      if (!frame) throw new DiagramInputError("`end` without an open block");
      target().push({ kind: "block", block: frame.block, sections: frame.sections });
    } else if ((match = BLOCK_OPEN.exec(line))) {
      if (frames.length >= MAX_DEPTH) {
        throw new DiagramLimitError(
          `Diagram too large: blocks nested more than ${String(MAX_DEPTH)} deep`,
        );
      }
      statementCount++;
      frames.push({
        block: (match[1] ?? "").toLowerCase() as BlockKind,
        sections: [{ label: (match[2] ?? "").trim(), statements: [] }],
      });
    } else {
      const frame = frames[frames.length - 1];
      const word = frame ? SECTION_WORD[frame.block] : undefined;
      const section = word?.exec(line);
      if (frame && section) {
        frame.sections.push({ label: (section[1] ?? "").trim(), statements: [] });
        continue;
      }
      match = MESSAGE.exec(line);
      if (!match) throw new DiagramInputError(`Unreadable sequence line: ${line.slice(0, 40)}`);
      const arrow = ARROWS.find(([token]) => token === match?.[2])?.[1];
      if (!arrow) throw new DiagramInputError("Unknown arrow");
      const from = known((match[1] ?? "").trim());
      const to = known((match[4] ?? "").trim());
      messageCount++;
      checkCount(messageCount, MAX_EDGES, "messages");
      const statement: SequenceStatement = {
        kind: "message",
        from,
        to,
        text: (match[5] ?? "").trim(),
        arrow,
        activation: match[3] === "+" ? "+" : match[3] === "-" ? "-" : undefined,
      };
      if (pendingCreate !== undefined) {
        if (pendingCreate !== to)
          throw new DiagramInputError("`create` must be followed by a message to it");
        statement.created = true;
        pendingCreate = undefined;
      }
      if (pendingDestroy !== undefined) {
        if (pendingDestroy !== to && pendingDestroy !== from) {
          throw new DiagramInputError("`destroy` must be followed by a message to or from it");
        }
        statement.destroyed = pendingDestroy;
        const gone = byId.get(pendingDestroy);
        if (gone) gone.destroyed = true;
        pendingDestroy = undefined;
      }
      push(statement);
    }
  }
  if (openBox !== undefined) throw new DiagramInputError("Unclosed box");
  if (frames.length > 0) throw new DiagramInputError("Unclosed block");
  if (pendingCreate !== undefined || pendingDestroy !== undefined) {
    throw new DiagramInputError("`create` or `destroy` without a message");
  }
  if (model.participants.length === 0)
    throw new DiagramInputError("A sequence diagram needs participants");
  return model;
}
