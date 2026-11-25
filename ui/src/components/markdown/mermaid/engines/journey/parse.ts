/**
 * User journey diagram parser. Follows Mermaid's userJourney page: `journey`, optional `title`,
 * `section <name>` groupings, and `Task name: <score>: <comma-separated actors>` lines (also
 * allowing omitted actor list). Score is a finite number in [1, 5]. Refuses styling/links/callbacks.
 */
import { linesOf } from "../../detect";
import { MAX_MEMBERS, MAX_NODES, checkCount } from "../../limits";
import { DiagramInputError, type PreparedSource } from "../../types";

/** One task step in a user journey with its satisfaction score, actors and section index. */
export interface JourneyTask {
  title: string;
  score: number;
  actors: string[];
  /** Index into `JourneyModel.sections`; 0 is the implicit default section. */
  section: number;
}

/** A named (or default empty) section in a user journey. */
export interface JourneySection {
  name: string;
}

/** Parsed user journey diagram ready for layout and drawing. */
export interface JourneyModel {
  title: string;
  sections: JourneySection[];
  actors: string[];
  tasks: JourneyTask[];
}

const REFUSED = /^(?:class|classDef|cssClass|click|link|callback|style)\b|:::/i;
const IGNORED = /^(?:accTitle|accDescr)\b/i;

function splitTaskLine(line: string): {
  title: string;
  scoreRaw: string;
  actorsRaw: string | undefined;
} {
  const withActors = /^(.+):\s*(-?\d+(?:\.\d+)?)\s*:\s*(.*)$/.exec(line);
  if (withActors) {
    return {
      title: (withActors[1] ?? "").trim(),
      scoreRaw: (withActors[2] ?? "").trim(),
      actorsRaw: (withActors[3] ?? "").trim(),
    };
  }
  const scoreOnly = /^(.+):\s*(-?\d+(?:\.\d+)?)\s*$/.exec(line);
  if (scoreOnly) {
    return {
      title: (scoreOnly[1] ?? "").trim(),
      scoreRaw: (scoreOnly[2] ?? "").trim(),
      actorsRaw: undefined,
    };
  }
  throw new DiagramInputError(`Unreadable journey line: ${line.slice(0, 40)}`);
}

/** Parse `journey` source (text or prepared lines); throws DiagramInputError on syntax/range errors. */
export function parseJourney(input: string | PreparedSource): JourneyModel {
  const [header = "", ...body] = linesOf(input, "journey");
  if (!/^journey\s*$/i.test(header.trim())) {
    throw new DiagramInputError("Unreadable journey header");
  }
  const model: JourneyModel = {
    title: "",
    sections: [{ name: "" }],
    actors: [],
    tasks: [],
  };
  const actorSet = new Set<string>();
  let inAccDescrBlock = false;

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
    if (/^title(?:\s|$)/i.test(line)) {
      const title = line.slice(5).trim();
      if (!title) throw new DiagramInputError("Journey title cannot be empty");
      model.title = title;
      continue;
    }
    if (/^section(?:\s|$)/i.test(line)) {
      const name = line.slice(7).trim();
      if (!name) throw new DiagramInputError("Journey section needs a name");
      model.sections.push({ name });
      checkCount(model.sections.length, MAX_NODES, "sections");
      continue;
    }
    if (!line.includes(":")) {
      throw new DiagramInputError(`Unreadable journey line: ${line.slice(0, 40)}`);
    }
    const { title, scoreRaw, actorsRaw } = splitTaskLine(line);
    if (!title) {
      throw new DiagramInputError(`Journey task needs a name: ${line.slice(0, 40)}`);
    }
    if (!/^-?\d+(?:\.\d+)?$/.test(scoreRaw)) {
      throw new DiagramInputError(`Invalid journey score: ${scoreRaw.slice(0, 20)}`);
    }
    const score = Number(scoreRaw);
    if (!Number.isFinite(score) || score < 1 || score > 5) {
      throw new DiagramInputError(`Journey score must be between 1 and 5: ${scoreRaw}`);
    }
    const actors: string[] = [];
    if (actorsRaw !== undefined && actorsRaw !== "") {
      for (const piece of actorsRaw.split(",")) {
        const actor = piece.trim();
        if (!actor) {
          throw new DiagramInputError(`Empty actor name in journey task: ${title.slice(0, 30)}`);
        }
        actors.push(actor);
        checkCount(actors.length, MAX_MEMBERS, "actors on a task");
        if (!actorSet.has(actor)) {
          actorSet.add(actor);
          model.actors.push(actor);
          checkCount(model.actors.length, MAX_NODES, "actors");
        }
      }
    }
    model.tasks.push({
      title,
      score,
      actors,
      section: model.sections.length - 1,
    });
    checkCount(model.tasks.length, MAX_NODES, "tasks");
  }

  if (model.tasks.length === 0) {
    throw new DiagramInputError("A journey diagram needs at least one task");
  }
  return model;
}
