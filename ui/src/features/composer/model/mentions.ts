/**
 * @ mention tokens, rows and picks: files from the core's name search and the other sessions working in the same
 * folder. Depends on core hits, the core store's session entries, the fuzzy matcher, the file chip builder and the
 * vocabulary; called by the composer hook.
 * Invariants: a pick replaces only the typed token and adds one chip (never a second one for the same file or
 * session); a session chip carries the core's bounded excerpt of that session (title, id, first message, last
 * answer; vault values withheld by the core) as data, never as a command.
 */
import type { FileHit, SessionExcerptView } from "../../../protocol/core";
import { sessionTitle, type SessionEntry } from "../../../state/sessionSummary";
import { TEXT, wording } from "../../../vocabulary";
import { absoluteFilePath, fileChip } from "../../files/model/context";
import type { App } from "../../shell/AppContext";
import { fuzzyScore, fuzzyTarget } from "./fuzzy";

/** The typed token, ending at the caret. */
export interface Mention {
  start: number;
  end: number;
  query: string;
}

/** Another session of the same folder, as the menu lists it. */
export interface SessionMention {
  id: string;
  title: string;
}

/** One row of the @ menu, files first. */
export type MentionRow =
  { kind: "file"; hit: FileHit } | { kind: "session"; session: SessionMention };

/** Most session rows shown. */
export const SESSION_ROWS = 5;

/** An @ at the start or after space, newline or ( opens a mention; whitespace ends it. */
export function mentionAt(text: string, caret: number): Mention | null {
  const match = /(?:^|[ \n(])@([^\s]*)$/.exec(text.slice(0, caret));
  if (match === null) return null;
  const query = match[1] ?? "";
  return { start: caret - query.length - 1, end: caret, query };
}

/** A core file path's name and relative folder, with the trailing slash shown in the menu. */
export function mentionName(path: string): { name: string; folder: string } {
  const split = path.lastIndexOf("/") + 1;
  return { name: path.slice(split), folder: path.slice(0, split) };
}

/**
 * The file rows: only names from this project's core results, at most eight, in the core's order (best match
 * first). A hit the fuzzy rule does not match (an answer for an older query) is dropped.
 */
export function mentionRows(hits: readonly FileHit[], project: string, query: string): FileHit[] {
  return hits
    .filter(
      (hit) =>
        hit.folder === project &&
        hit.line === null &&
        fuzzyScore(fuzzyTarget(hit.path, query), query) !== null,
    )
    .slice(0, 8);
}

function sameFolder(a: string, b: string): boolean {
  const plain = (path: string) => path.replaceAll("\\", "/").replace(/\/+$/, "");
  return plain(a) === plain(b);
}

function number(value: unknown): number {
  return typeof value === "number" ? value : 0;
}

/**
 * The session rows: the other top-level sessions working in `project` (not `current`, not a subagent's, not
 * archived) whose title matches `query`, best match first, then the most recently updated; at most
 * [`SESSION_ROWS`]. A query with a `/` names a file path, so it lists no session.
 */
export function sessionRows(
  sessions: Readonly<Record<string, SessionEntry>>,
  project: string,
  current: string,
  query: string,
): SessionMention[] {
  if (query.includes("/")) return [];
  return Object.values(sessions)
    .flatMap((entry) => {
      const s = entry.session;
      const cwd = s["cwd"];
      const parent = s["parentSessionId"];
      if (
        entry.id === current ||
        entry.ownedByParent === true ||
        (typeof parent === "string" && parent !== "") ||
        (s["archivedAt"] ?? null) !== null ||
        typeof cwd !== "string" ||
        !sameFolder(cwd, project)
      )
        return [];
      const title = sessionTitle(s);
      const score = fuzzyScore(title, query);
      return score === null
        ? []
        : [{ id: entry.id, title, score, updated: number(s["updatedAt"]) }];
    })
    .sort((a, b) => b.score - a.score || b.updated - a.updated)
    .slice(0, SESSION_ROWS)
    .map(({ id, title }) => ({ id, title }));
}

/** Replaces just the mention and inserts one whole-file reference through the existing facade. */
export function chooseMention(app: App, sessionId: string, mention: Mention, hit: FileHit): number {
  const { name } = mentionName(hit.path);
  const chip = fileChip({
    path: absoluteFilePath(hit.path, hit.folder),
    project: hit.folder,
    name,
    unsaved: false,
  });
  const duplicate = (app.composer.store.get().context[sessionId] ?? []).some(
    (item) => item.ref?.path === chip.ref.path && item.ref.start === 0 && item.ref.end === 0,
  );
  if (!duplicate) app.composer.actions?.insertContext(sessionId, chip);
  return replaceToken(app, sessionId, mention, name);
}

/** The chip text: the session's title and id, then its first message and last answer when the excerpt has them. */
export function sessionChipText(id: string, excerpt: SessionExcerptView): string {
  return [
    wording(TEXT.mentionSessionText, { title: excerpt.title, id }),
    excerpt.firstUser === null
      ? null
      : wording(TEXT.mentionSessionFirst, { text: excerpt.firstUser }),
    excerpt.lastAnswer === null
      ? null
      : wording(TEXT.mentionSessionLast, { text: excerpt.lastAnswer }),
  ]
    .filter((line): line is string => line !== null)
    .join("\n");
}

/** The core's bounded excerpt of `id` (title, first message, last answer; vault values withheld), or `null`. */
async function readExcerpt(app: App, id: string): Promise<SessionExcerptView | null> {
  try {
    const answer = await app.core?.run({
      type: "sessionTools",
      request: { op: "excerpt", sessionId: id },
    });
    return answer?.type === "sessionTools" && answer.result.kind === "excerpt"
      ? answer.result.excerpt
      : null;
  } catch {
    return null;
  }
}

/**
 * Replaces just the mention with the session's title at once, then adds one chip carrying the session's bounded
 * excerpt (the title and id alone when it cannot be read), the way "Point Vibe at this" adds context: data sent with
 * the next message. The same session twice adds one chip.
 */
export function chooseSession(
  app: App,
  sessionId: string,
  mention: Mention,
  session: SessionMention,
): number {
  const marker = wording(TEXT.mentionSessionText, { title: session.title, id: session.id });
  const id = `(session id ${session.id})`;
  const held = () =>
    (app.composer.store.get().context[sessionId] ?? []).some(
      (item) =>
        item.source === "session" &&
        (item.text === marker || item.text.split("\n")[0]?.endsWith(`${id}.`) === true),
    );
  if (!held())
    void readExcerpt(app, session.id).then((excerpt) => {
      if (held()) return;
      app.composer.actions?.insertContext(sessionId, {
        source: "session",
        label: wording(TEXT.mentionSessionLabel, { title: session.title }),
        text: excerpt === null ? marker : sessionChipText(session.id, excerpt),
      });
    });
  return replaceToken(app, sessionId, mention, session.title);
}

function replaceToken(app: App, sessionId: string, mention: Mention, text: string): number {
  const draft = app.composer.store.get().drafts[sessionId] ?? "";
  app.composer.setDraft(sessionId, draft.slice(0, mention.start) + text + draft.slice(mention.end));
  return mention.start + text.length;
}
