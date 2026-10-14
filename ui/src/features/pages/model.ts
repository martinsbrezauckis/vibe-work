/**
 * Project tab presentation model: plan progress from the page's steps, Vibe's todo list as a live
 * overlay, the "Plan not updated" rule, the context chips "Point Vibe at this", page comments, "update the index"
 * and "migrate the handbook" put into the composer, and the fixed prompts the Projects overview sends. Pure
 * functions, so the components only render.
 * Depends on: protocol/core (Step, Comment, Picked, PickedSection, Finding), transcript/model/todos (Todo). Called
 * by: the Project tab components, the Projects overview, store.ts, point.ts, UI tests.
 * Invariants:
 * - The plan's status lives in the page (PROJECT_PAGES "One source, many views"); todos only overlay it while a turn
 *   runs. "Plan not updated" is shown only when no turn runs and the todo list says MORE steps are done than the
 *   page (the page is behind); never the other way round.
 * - Every chip is data: its text quotes the page/terminal/element, never starts a command (the composer sends chips
 *   as resource blocks; "sent context is never a command").
 */
import type { Comment, Finding, Picked, PickedSection, Step } from "../../protocol/core";
import type { Todo } from "../transcript/model/todos";

/** Finished (done or skipped) and all steps, and the step in progress. */
export interface PlanProgress {
  done: number;
  total: number;
  /** Index of the first `doing` step, else -1. */
  doing: number;
}

/** True for a finished step (Handbook Format 1.1: progress counts done and skipped steps). */
export function finished(status: string): boolean {
  return status === "done" || status === "skipped";
}

/** Progress of a plan page's steps. */
export function planProgress(steps: readonly Pick<Step, "status">[]): PlanProgress {
  return {
    done: steps.filter((s) => finished(s.status)).length,
    total: steps.length,
    doing: steps.findIndex((s) => s.status === "doing"),
  };
}

/** Our step state for a todo status. */
export function todoState(status: string): string {
  switch (status) {
    case "completed":
      return "done";
    case "in_progress":
      return "doing";
    case "cancelled":
      return "skipped";
    default:
      return "todo";
  }
}

const norm = (text: string) => text.toLowerCase().replace(/\s+/g, " ").trim();

/**
 * The todo matching each step (by id, else by the same text), in step order; `null` where none matches.
 * With no id or text match at all, todos match steps by position when the counts are equal.
 */
export function matchTodos(steps: readonly Step[], todos: readonly Todo[]): Array<Todo | null> {
  const byId = steps.map((s) => todos.find((t) => t.id === s.id) ?? null);
  const byText = steps.map(
    (s, i) => byId[i] ?? todos.find((t) => norm(t.content) === norm(s.text)) ?? null,
  );
  if (byText.some((t) => t !== null)) return byText;
  return todos.length === steps.length ? [...todos] : byText.map(() => null);
}

/** Index of the step the todo list says is in progress (live overlay while a turn runs), else -1. */
export function liveStep(steps: readonly Step[], todos: readonly Todo[] | null): number {
  if (todos === null) return -1;
  return matchTodos(steps, todos).findIndex((t) => t !== null && t.status === "in_progress");
}

/**
 * The steps as the side panel shows them (the panel said "0 of 9 done" while the todo list
 * said 6/9): while a turn runs, a step the page does not mark finished takes "done" / "skipped" from its matched todo
 * (`fromTodo`), because the agent updates the page at the end of the turn. Without a running turn, or without a
 * match, the page's own status stands (the page stays the one source; "Plan not updated" covers the end of a turn).
 */
export function overlaySteps(
  steps: readonly Step[],
  todos: readonly Todo[] | null,
  running: boolean,
): Array<Step & { fromTodo: boolean }> {
  const matched = running && todos !== null ? matchTodos(steps, todos) : [];
  return steps.map((step, i) => {
    const todo = matched[i] ?? null;
    const state = todo === null ? null : todoState(todo.status);
    return !finished(step.status) && state !== null && finished(state)
      ? { ...step, status: state, fromTodo: true }
      : { ...step, fromTodo: false };
  });
}

/** What "Plan not updated" says: the todo list's done count against the page's. */
export interface PlanMismatch {
  pageDone: number;
  todoDone: number;
  total: number;
}

/**
 * The end-of-turn check: `null` while a turn runs, without a todo list, or when the page is not behind the todo list.
 * Matched steps compare one by one; without any match the done counts compare.
 */
export function planMismatch(
  steps: readonly Step[],
  todos: readonly Todo[] | null,
  running: boolean,
): PlanMismatch | null {
  if (running || todos === null || todos.length === 0 || steps.length === 0) return null;
  const progress = planProgress(steps);
  const matched = matchTodos(steps, todos);
  const anyMatch = matched.some((t) => t !== null);
  const todoDone = anyMatch
    ? matched.filter((t, i) => t?.status === "completed" || steps[i]?.status === "skipped").length
    : todos.filter((t) => t.status === "completed" || t.status === "cancelled").length;
  const behind = anyMatch
    ? matched.some(
        (t, i) =>
          t?.status === "completed" &&
          steps[i]?.status !== "done" &&
          steps[i]?.status !== "skipped",
      )
    : todoDone > progress.done;
  return behind ? { pageDone: progress.done, todoDone, total: progress.total } : null;
}

/**
 * The folder the chips name pages by: the handbook folder (`docs/handbook`), made absolute when the
 * Project tab shows the handbook of another folder than the one the session works in (its project's main folder),
 * so the agent reads the files the user pointed at and not its own folder's.
 */
export function chipFolder(
  hub: { project: string; settings: { folder: string } } | undefined,
  cwd: string | null,
): string {
  const folder = hub?.settings.folder ?? "docs/handbook";
  const bare = (p: string) => p.replace(/[\\/]+$/, "");
  if (hub === undefined || cwd === null || bare(hub.project) === bare(cwd)) return folder;
  return `${bare(hub.project)}/${folder}`;
}

/** A context chip for the composer (`insertContext`). */
export interface Chip {
  label: string;
  source: string;
  text: string;
}

const clip = (text: string, max: number) => (text.length > max ? `${text.slice(0, max)}…` : text);

/** The chip a page comment becomes ("Send to Vibe"): cites the page and the section id. */
export function commentChip(pagesFolder: string, comment: Comment): Chip {
  const lines = [
    `Comment on the page ${pagesFolder}/${comment.page}, section #${comment.section} ("${comment.sectionLabel}"):`,
  ];
  if (comment.quote !== null) lines.push(`Quoted from the page: "${clip(comment.quote, 1500)}"`);
  lines.push(`Comment: ${comment.text}`);
  return { label: `Page comment · ${comment.section}`, source: "page", text: lines.join("\n") };
}

/** The chip "Point Vibe at this" makes from a page section. */
export function sectionChip(pagesFolder: string, picked: PickedSection): Chip {
  const quote = picked.quote ?? picked.text;
  return {
    label: `Page section · ${picked.section}`,
    source: "page",
    text: [
      `Page ${pagesFolder}/${picked.page}, section #${picked.section} ("${picked.label}"):`,
      `"${clip(quote, 1800)}"`,
    ].join("\n"),
  };
}

/** The chip "Point Vibe at this" makes from an element of the session's preview. */
export function elementChip(picked: Picked): Chip {
  const lines = [
    `Element in the preview at ${picked.url}`,
    `Path: ${picked.path}`,
    `Role: ${picked.role}`,
  ];
  if (picked.quote !== null) lines.push(`Selected text: "${clip(picked.quote, 1500)}"`);
  else if (picked.text !== "") lines.push(`Text: "${clip(picked.text, 1500)}"`);
  return { label: `Preview element · ${picked.tag}`, source: "browser", text: lines.join("\n") };
}

/** The chip "Point Vibe at this" makes from the terminal's selection. */
export function terminalChip(selection: string): Chip {
  return {
    label: "Terminal selection",
    source: "terminal",
    text: `Selected in the terminal:\n${clip(selection, 8000)}`,
  };
}

/** The chip "Ask Vibe to update the page" inserts (the user still sends it). */
export function mismatchChip(pagesFolder: string, path: string, mismatch: PlanMismatch): Chip {
  return {
    label: "Plan not updated",
    source: "page",
    text:
      `The plan page ${pagesFolder}/${path} says ${String(mismatch.pageDone)} of ${String(mismatch.total)} steps ` +
      `are done; your todo list says ${String(mismatch.todoDone)}. Please update the step statuses on the page.`,
  };
}

/** The first message "Migrate" sends for a handbook in the earlier format (docs/pages/, `vw-*` names). */
export const MIGRATE_HANDBOOK_PROMPT =
  "Migrate this project's handbook from docs/pages/ to docs/handbook/ (Project Handbook Format 1.1, section 11): " +
  "use the handbook skill, move the files, update every page head and link, write index.html, then run handbook_check.";

/** One finding as a line of text (`HB-W03 index.html: plans/x.html is not linked from the index`). */
export function findingLine(f: Finding): string {
  return `${f.rule} ${f.page}: ${f.message}`;
}

/** The chip "Ask Vibe to fix the handbook": the errors and warnings to fix (the user still sends it). */
export function fixChip(folder: string, findings: readonly Finding[]): Chip {
  const shown = findings.filter((f) => f.severity !== "info").slice(0, 40);
  return {
    label: "Handbook findings",
    source: "page",
    text: [
      `Vibe Work's check of the handbook in ${folder}/ (Project Handbook Format 1.1) reports:`,
      ...shown.map((f) => `- ${f.severity} ${findingLine(f)}`),
      "Please fix them (keep index.html current), then run handbook_check.",
    ].join("\n"),
  };
}

/** The chip "Ask Vibe to migrate": the handbook is in the earlier format (the user still sends it). */
export function migrationChip(): Chip {
  return { label: "Migrate the handbook", source: "page", text: MIGRATE_HANDBOOK_PROMPT };
}
