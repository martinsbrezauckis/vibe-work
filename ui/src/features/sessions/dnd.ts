/**
 * Drag and drop in the session list: what a dragged session row or project carries, and the checks a
 * drop target makes. Uses own MIME types
 * only, no `text/plain` (so a row dropped on the composer or any text field inserts nothing; the composer's drop takes
 * `Files` only), the type list checked during `dragover` (the data itself is unreadable until `drop`).
 * Adapted from ShellX.
 * Depends on: nothing. Called by: SessionRow, ProjectGroupView, UnfiledSection.
 * Invariants: a drop is acted on only when it carries our MIME type and parses; a session drag carries the row and
 * its nested children (they move together); malformed data is ignored, never thrown.
 */

/** A dragged session row: its id and its children's ids. */
export const SESSION_MIME = "application/x-vibe-work-sessions";
/** A dragged project (reordering). */
export const PROJECT_MIME = "application/x-vibe-work-project";

/** The part of `DataTransfer` the list uses (a plain object in tests). */
export interface Transfer {
  readonly types: readonly string[] | DOMStringList;
  getData(type: string): string;
  setData(type: string, data: string): void;
  dropEffect: string;
  effectAllowed: string;
}

function has(transfer: Transfer, type: string): boolean {
  return Array.from(transfer.types as ArrayLike<string>).includes(type);
}

/** True while sessions are dragged (checked during `dragover`, when only the types are readable). */
export function carriesSessions(transfer: Transfer): boolean {
  return has(transfer, SESSION_MIME);
}

/** True while a project is dragged. */
export function carriesProject(transfer: Transfer): boolean {
  return has(transfer, PROJECT_MIME);
}

/** Starts a session drag. */
export function dragSessions(transfer: Transfer, ids: readonly string[]): void {
  transfer.setData(SESSION_MIME, JSON.stringify(ids));
  transfer.effectAllowed = "move";
}

/** Starts a project drag. */
export function dragProject(transfer: Transfer, id: string): void {
  transfer.setData(PROJECT_MIME, id);
  transfer.effectAllowed = "move";
}

/** The dropped session ids (empty when the data is not ours or malformed). */
export function droppedSessions(transfer: Transfer): string[] {
  try {
    const value: unknown = JSON.parse(transfer.getData(SESSION_MIME));
    return Array.isArray(value)
      ? value.filter((v): v is string => typeof v === "string" && v !== "").slice(0, 500)
      : [];
  } catch {
    return [];
  }
}

/** The dropped project id, or `null`. */
export function droppedProject(transfer: Transfer): string | null {
  const id = transfer.getData(PROJECT_MIME);
  return id === "" ? null : id;
}

/**
 * True when the pointer left `element` for good (`dragleave` fires for every child it crosses). Chromium often
 * gives `dragleave` no `relatedTarget` (found by projects_e2e: the drop mark vanished while the pointer was still
 * over "Not in a project"), so without one the pointer position decides: still inside the element's box = not left.
 */
export function leftFor(
  element: Element,
  related: EventTarget | null,
  at?: { clientX: number; clientY: number },
): boolean {
  if (related instanceof Node) return !element.contains(related);
  if (at === undefined || !Number.isFinite(at.clientX) || !Number.isFinite(at.clientY)) return true;
  const box = element.getBoundingClientRect();
  const inside =
    at.clientX > box.left &&
    at.clientX < box.right &&
    at.clientY > box.top &&
    at.clientY < box.bottom;
  return !inside;
}
