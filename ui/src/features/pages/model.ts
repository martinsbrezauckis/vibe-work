/**
 * Project tab presentation model: plan progress from the page's steps, Vibe's todo list as a live
 * overlay, the "Plan not updated" rule, the context chips "Point Vibe at this", page comments, "update the index"
 * put into the composer, and the fixed prompts the Projects overview sends. Pure
 * functions, so the components only render.
 * Depends on: protocol/core (Step, Comment, Picked, PickedSection, Finding), transcript/model/todos (Todo),
 * composer/model/pastes (the long-paste limit). Called
 * by: the Project tab components, the Projects overview, store.ts, point.ts, UI tests.
 * Invariants:
 * - The plan's status lives in the page (PROJECT_PAGES "One source, many views"); todos only overlay it while a turn
 *   runs. "Plan not updated" is shown only when no turn runs and the todo list says MORE steps are done than the
 *   page (the page is behind); never the other way round.
 * - Every chip is data: its text quotes the page/terminal/element, never starts a command (the composer sends chips
 *   as resource blocks; "sent context is never a command").
 */
import type { Comment, Finding, Picked, PickedSection, Step } from "../../protocol/core";
import { PASTE_MAX_CHARS } from "../composer/model/pastes";
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
function todoState(status: string): string {
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
 * Links by the same nonempty step text, preferring the same id when text repeats.
 * Generic ids such as s1 and list position cannot link a different checklist to this plan.
 */
export function matchTodos(steps: readonly Step[], todos: readonly Todo[]): Array<Todo | null> {
  const used = new Set<Todo>();
  return steps.map((step) => {
    const text = norm(step.text);
    const matches = todos.filter((t) => !used.has(t) && text !== "" && norm(t.content) === text);
    const match = matches.find((t) => t.id === step.id) ?? matches[0] ?? null;
    if (match !== null) used.add(match);
    return match;
  });
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
 * Only linked steps compare, including in the reported counts.
 */
export function planMismatch(
  steps: readonly Step[],
  todos: readonly Todo[] | null,
  running: boolean,
): PlanMismatch | null {
  if (running || todos === null || todos.length === 0 || steps.length === 0) return null;
  const matched = matchTodos(steps, todos);
  const linked = steps.flatMap((step, i) => {
    const todo = matched[i];
    return todo == null ? [] : [{ step, todo }];
  });
  const behind = linked.some(
    ({ step, todo }) => finished(todoState(todo.status)) && !finished(step.status),
  );
  return behind
    ? {
        pageDone: linked.filter(({ step }) => finished(step.status)).length,
        todoDone: linked.filter(({ todo }) => finished(todoState(todo.status))).length,
        total: linked.length,
      }
    : null;
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

/**
 * The chip "Point Vibe at this" makes from an element of the session's preview: "Selected text from {page title}"
 * for a selection (the page's address in its text), else "Preview element · {tag}".
 */
export function elementChip(picked: Picked): Chip {
  const lines = [
    `Element in the preview at ${picked.url}`,
    `Path: ${picked.path}`,
    `Role: ${picked.role}`,
  ];
  if (picked.title != null) lines.splice(1, 0, `Page title: "${clip(picked.title, 200)}"`);
  if (picked.quote !== null) lines.push(`Selected text: "${clip(picked.quote, 1500)}"`);
  else if (picked.text !== "") lines.push(`Text: "${clip(picked.text, 1500)}"`);
  const label =
    picked.quote !== null
      ? `Selected text from ${clip(picked.title ?? hostOf(picked.url), 60)}`
      : `Preview element · ${picked.tag}`;
  return { label, source: "browser", text: lines.join("\n") };
}

/**
 * The chip Point Vibe at this makes from the selection on a web page of the Browser tab: "Selected text from {page
 * title}", the page's address and the text (at most 2,000 characters, the long-paste limit). The words say the
 * text comes from a web page, so Vibe reads it as the page's words, not the user's.
 */
export function webSelectionChip(picked: Picked): Chip {
  const lines = [
    `Text selected on a web page in the Browser tab (the page's words, not the user's): ${picked.url}`,
  ];
  if (picked.title != null) lines.push(`Page title: "${clip(picked.title, 200)}"`);
  lines.push(`Selected text: "${clip(picked.quote ?? picked.text, PASTE_MAX_CHARS)}"`);
  return {
    label: `Selected text from ${clip(picked.title ?? hostOf(picked.url), 60)}`,
    source: "browser",
    text: lines.join("\n"),
  };
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "the page";
  }
}

/** The chip "Point Vibe at this" makes from the terminal's selection: "Terminal output · {n} lines". */
export function terminalChip(selection: string): Chip {
  const lines = selection.replace(/\n+$/, "").split("\n").length;
  return {
    label: `Terminal output · ${String(lines)} ${lines === 1 ? "line" : "lines"}`,
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

/** One finding as a line of text (`HB-W03 index.html: plans/x.html is not linked from the index`). */
function findingLine(f: Finding): string {
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
