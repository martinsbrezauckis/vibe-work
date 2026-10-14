/**
 * The session list's one drop controller: every drag over the list is decided here, on the list column,
 * and the current drop mark lives in one small store the groups render (`data-drop`).
 *
 * Why one controller and not a handler per group (the ShellX shape): per-group marks pair `dragenter` with
 * `dragleave`, and Chromium sends those with a missing `relatedTarget` and in varying order, so a mark vanished or
 * stayed depending on timing (projects_e2e). Here the column cancels `dragenter` and `dragover` (an
 * element that lets `dragenter` pass is not the drop target and gets no `dragover`), finds the group under the
 * pointer from the event's target (`[data-drop-key]`), and a leave only clears the mark when the pointer left the
 * column itself.
 * Depends on: createStore, dnd (MIME types), state/projectGroups. Called by: SessionList (handlers), ProjectGroupView
 * and UnfiledSection (`useDropMark`).
 * Invariants: sessions drop on a project or "Not in a project" (never on a folder group the core has not made yet);
 * projects drop on another project, before or after it by the pointer's half; only our MIME types are accepted; the
 * mark is cleared on drop, on drag end anywhere, and when the drag leaves the column.
 */
import { type DragEvent, useEffect } from "react";

import { createStore, useStore } from "../../state/createStore";
import type { ProjectGroups } from "../../state/projectGroups";
import { carriesProject, carriesSessions, droppedProject, droppedSessions, leftFor } from "./dnd";

/** The collapse and drop key of "Not in a project" (no project id starts with `_`). */
export const UNFILED_KEY = "__unfiled__";

/** Where a drop would land now. */
export interface DropMark {
  /** The group's key (`data-drop-key`). */
  key: string;
  edge: "into" | "before" | "after";
}

const marks = createStore<DropMark | null>(null);

function setMark(next: DropMark | null): void {
  const now = marks.get();
  if (now?.key === next?.key && now?.edge === next?.edge) return;
  marks.set(() => next);
}

/** True while sessions are being dragged (the empty "Not in a project" shows up as a target). */
const dragging = createStore(false);

/** A session row calls this when its drag starts (the row stops the event, so the document never sees it). */
export function beginSessionDrag(): void {
  dragging.set(() => true);
}

/** True while a session drag is in progress anywhere in the document. */
export function useSessionDragging(): boolean {
  return useStore(dragging, (d) => d);
}

/** The drop mark of the group `key` (for its `data-drop` attribute), or `undefined`. */
export function useDropMark(key: string): DropMark["edge"] | undefined {
  return useStore(marks, (m) => (m?.key === key ? m.edge : undefined));
}

interface Target {
  element: Element;
  key: string;
  /** The project id (`null` for a folder group the core has not made yet). */
  id: string | null;
}

function targetOf(event: DragEvent<HTMLElement>): Target | null {
  const node = event.target;
  const element = node instanceof Element ? node.closest("[data-drop-key]") : null;
  if (element === null) return null;
  const key = element.getAttribute("data-drop-key") ?? "";
  const id = element.getAttribute("data-drop-id");
  return { element, key, id: id === "" ? null : id };
}

/** The mark a drag over `event`'s target would get, `null` when it cannot land there. */
function decide(event: DragEvent<HTMLElement>): (DropMark & { id: string | null }) | null {
  const target = targetOf(event);
  if (target === null) return null;
  if (carriesSessions(event.dataTransfer)) {
    return target.id !== null || target.key === UNFILED_KEY
      ? { key: target.key, edge: "into", id: target.id }
      : null;
  }
  if (carriesProject(event.dataTransfer) && target.id !== null) {
    const box = target.element.getBoundingClientRect();
    const upper = event.clientY < box.top + box.height / 2;
    return { key: target.key, edge: upper ? "before" : "after", id: target.id };
  }
  return null;
}

/**
 * The column's drag handlers. `order` gives the project ids in the user's order (for "after" drops). Also clears
 * the mark on any `dragend` or `drop` in the document while mounted.
 */
export function useListDrop(groups: ProjectGroups | null, order: () => readonly string[]) {
  useEffect(() => {
    const clear = () => {
      setMark(null);
      dragging.set(() => false);
    };
    document.addEventListener("dragend", clear, true);
    document.addEventListener("drop", clear);
    return () => {
      document.removeEventListener("dragend", clear, true);
      document.removeEventListener("drop", clear);
    };
  }, []);
  const over = (event: DragEvent<HTMLElement>) => {
    const mark = groups === null ? null : decide(event);
    if (mark === null) {
      setMark(null);
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setMark({ key: mark.key, edge: mark.edge });
  };
  return {
    onDragEnter: over,
    onDragOver: over,
    onDragLeave: (event: DragEvent<HTMLElement>) => {
      if (leftFor(event.currentTarget, event.relatedTarget, event)) setMark(null);
    },
    onDrop: (event: DragEvent<HTMLElement>) => {
      const mark = groups === null ? null : decide(event);
      setMark(null);
      if (mark === null || groups === null) return;
      event.preventDefault();
      if (carriesSessions(event.dataTransfer)) {
        const ids = droppedSessions(event.dataTransfer);
        if (ids.length > 0) void groups.move(ids, mark.key === UNFILED_KEY ? null : mark.id);
        return;
      }
      const moving = droppedProject(event.dataTransfer);
      const id = mark.id;
      if (moving === null || id === null || moving === id) return;
      const rest = order().filter((p) => p !== moving);
      const at = rest.indexOf(id);
      const before = mark.edge === "before" ? id : (rest[at + 1] ?? null);
      void groups.place(moving, before);
    },
  };
}
