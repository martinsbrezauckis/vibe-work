/**
 * Rows that move instead of jumping (feel item 10): after every render of the session list, rows whose
 * place changed glide from where they were (240 ms, FLIP: measure, move, animate the transform back to zero). A
 * session that now needs you appears in "Needs you" sliding from where its row was and lands with one orange wash.
 * Depends on: react, the DOM (`data-session-id` rows in the list). Called by: SessionList.
 * Invariants: positions are measured inside the scroll container (scrolling never counts as moving); nothing moves
 * while filtering, on clearing, while dragged (drop code reads row boxes), or under reduced motion. Filter changes
 * cancel movement before measuring, so old transforms never inflate the scroller during a search.
 */
import { type RefObject, useLayoutEffect, useRef } from "react";

function keyOf(el: HTMLElement): string {
  const id = el.dataset["sessionId"] ?? "";
  if (el.classList.contains("needs-row")) return `needs:${id}`;
  return el.closest(".pinned") === null ? `list:${id}` : `pinned:${id}`;
}

function reduced(): boolean {
  return (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** Runs the motion for the list inside `container`. */
export function useListMotion(container: RefObject<HTMLElement | null>, filter: string): void {
  const last = useRef(new Map<string, number>());
  const previousFilter = useRef(filter);
  const animations = useRef<Animation[]>([]);
  useLayoutEffect(
    () => () => {
      for (const animation of animations.current) animation.cancel();
    },
    [],
  );
  useLayoutEffect(() => {
    const root = container.current;
    if (root === null) return;
    const filtering = filter.trim() !== "" || previousFilter.current !== filter;
    previousFilter.current = filter;
    for (const animation of animations.current) animation.cancel();
    animations.current = [];
    const base = root.getBoundingClientRect().top - root.scrollTop;
    const rows = Array.from(root.querySelectorAll<HTMLElement>("[data-session-id]")).filter(
      (el) => el.closest("[hidden]") === null,
    );
    const now = new Map<string, number>();
    for (const el of rows) now.set(keyOf(el), el.getBoundingClientRect().top - base);
    const before = last.current;
    last.current = now;
    if (filtering || before.size === 0 || reduced() || root.querySelector(".is-dragging") !== null)
      return;
    for (const el of rows) {
      if (typeof el.animate !== "function") continue;
      const key = keyOf(el);
      const top = now.get(key) ?? 0;
      let from = before.get(key);
      const arrived = from === undefined && key.startsWith("needs:");
      if (arrived) {
        const id = key.slice("needs:".length);
        from = before.get(`list:${id}`) ?? before.get(`pinned:${id}`);
      }
      if (from === undefined || Math.abs(from - top) < 1) {
        if (arrived) el.classList.add("landed");
        continue;
      }
      animations.current.push(
        el.animate([{ transform: `translateY(${String(from - top)}px)` }, { transform: "none" }], {
          duration: 240,
          easing: "cubic-bezier(.16, 1, .3, 1)",
        }),
      );
      if (arrived) {
        el.classList.remove("landed");
        el.classList.add("landed");
      }
    }
  });
}
