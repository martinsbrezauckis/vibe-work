/**
 * Stick-to-bottom with a "Jump to latest" pill, driven by explicit user intent. Deriving "pinned" from `scrollTop` alone makes the pill flicker on every streamed chunk, because new
 * content grows the list before the follow-scroll runs. The view un-pins only after a
 * wheel-up, PageUp / ArrowUp / Home or an upward touch drag AND a position more than 200 px from the
 * bottom; reaching the bottom (within 8 px) re-pins. Every row change and every size change follows, including a message growing in place.
 * No smooth scrolling.
 * Adapted from ShellX.
 * Depends on: react, virtua's `VListHandle`. Called by: Transcript.
 */
import {
  type KeyboardEvent,
  type RefObject,
  type TouchEvent,
  type WheelEvent,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { VListHandle } from "virtua";

/** Distance from the bottom that still counts as "at the bottom". */
const AT_BOTTOM_PX = 8;
/** Distance the user must have scrolled away (on purpose) before the pill shows. */
const AWAY_PX = 200;

/** The follow state and the handlers to attach. */
export function useFollow(list: RefObject<VListHandle | null>, count: number, version: unknown) {
  const [pinned, setPinned] = useState(true);
  const userUp = useRef(false);
  const touchY = useRef(0);

  const toEnd = useCallback(() => {
    if (count > 0) list.current?.scrollToIndex(count - 1, { align: "end" });
  }, [list, count]);

  // Follow new rows and rows that changed while pinned.
  useLayoutEffect(() => {
    if (pinned) toEnd();
  }, [pinned, toEnd, version]);

  const onScroll = useCallback(
    (offset: number) => {
      const handle = list.current;
      if (handle === null) return;
      const distance = handle.scrollSize - offset - handle.viewportSize;
      if (distance < AT_BOTTOM_PX) {
        userUp.current = false;
        setPinned(true);
      } else if (userUp.current && distance > AWAY_PX) {
        setPinned(false);
      }
    },
    [list],
  );

  return {
    pinned,
    onScroll,
    /** Sizes changed (a row grew while streaming): keep the end in view while pinned. */
    onResize: () => {
      if (pinned) toEnd();
    },
    jump: () => {
      userUp.current = false;
      setPinned(true);
      toEnd();
    },
    intent: {
      onWheel: (event: WheelEvent) => {
        if (event.deltaY < 0) userUp.current = true;
      },
      onKeyDown: (event: KeyboardEvent) => {
        if (event.key === "ArrowUp" || event.key === "PageUp" || event.key === "Home")
          userUp.current = true;
      },
      onTouchStart: (event: TouchEvent) => {
        touchY.current = event.touches[0]?.clientY ?? 0;
      },
      onTouchMove: (event: TouchEvent) => {
        if ((event.touches[0]?.clientY ?? 0) > touchY.current + 8) userUp.current = true;
      },
    },
  };
}
