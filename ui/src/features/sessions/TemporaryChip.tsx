/**
 * The session header's "Temporary" chip: shown while the open session is a temporary chat, with
 * what happens to it in words.
 * Depends on: AppContext, state/createStore, state/sessionTools. Called by: shell/Content (the session header).
 * Invariants: renders nothing for a normal session. A narrow header (a side panel open, or a chat area under
 * 1040 px) shows only "Temporary"; the whole explanation is the chip's tooltip, reachable without a mouse: the chip
 * is in the Tab order, the explanation is its accessible name and the visible words stay exposed (NVDA/WebView2
 * reads those words for a focused note), the tooltip shows while
 * the chip is hovered or focused, and Esc hides it while focus stays. The tooltip stays inside the session header
 * (`tooltipPlace`): a side panel beside it is a native view painted above the page, which would cut it off.
 */
import { Badge } from "../../components/Controls";
import { type ReactElement, useId, useLayoutEffect, useRef, useState } from "react";

import { useStore } from "../../state/createStore";
import { NO_SESSION_TOOLS } from "../../state/sessionTools";
import { useApp } from "../shell/AppContext";

/** What a temporary chat keeps and loses: the chip's tooltip. */
const EXPLANATION =
  "Temporary chat: deleted when you quit. Files in a folder you chose stay changed; the chat's own folder is removed.";

/** Smallest gap between the tooltip and the edge of the header it must stay inside, in CSS pixels. */
const EDGE = 8;
/** The tooltip's widest size (as in the stylesheet). */
const WIDEST = 320;

/** Where the tooltip goes: its left offset from the chip and its widest size, so it fits inside `area`. */
export function tooltipPlace(
  anchor: { left: number },
  tipWidth: number,
  area: { left: number; right: number },
): { left: number; maxWidth: number } {
  const maxWidth = Math.max(0, Math.min(WIDEST, area.right - area.left - 2 * EDGE));
  const width = Math.min(tipWidth, maxWidth);
  const start = Math.max(area.left + EDGE, Math.min(anchor.left, area.right - EDGE - width));
  return { left: start - anchor.left, maxWidth };
}

/** The chip, or nothing. */
export function TemporaryChip({ sessionId }: { sessionId: string }): ReactElement | null {
  const { sessionTools } = useApp();
  const temporary = useStore(sessionTools?.store ?? NO_SESSION_TOOLS, (s) =>
    s.temporary.includes(sessionId),
  );
  const tip = useId();
  const [shown, setShown] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const tooltip = useRef<HTMLSpanElement>(null);
  const [place, setPlace] = useState<{ left: number; maxWidth: number } | null>(null);
  // Measured once the tooltip shows (it needs its own width), before the browser paints it.
  useLayoutEffect(() => {
    const box = anchor.current;
    const tipBox = tooltip.current;
    if (!shown || box === null || tipBox === null) return;
    const area = (box.closest(".shead") ?? document.documentElement).getBoundingClientRect();
    setPlace(tooltipPlace(box.getBoundingClientRect(), tipBox.getBoundingClientRect().width, area));
  }, [shown]);
  if (!temporary) return null;
  return (
    <span
      ref={anchor}
      className="temporary-explained"
      data-shown={shown ? "true" : "false"}
      onMouseEnter={() => {
        setShown(true);
      }}
      onMouseLeave={() => {
        setShown(false);
      }}
    >
      <Badge
        presentation="temporary"
        data-testid="temporary-chip"
        tabIndex={0}
        // A focusable note: its visible words remain available to screen readers alongside the full name.
        role="note"
        aria-label={EXPLANATION}
        onFocus={() => {
          setShown(true);
        }}
        onBlur={() => {
          setShown(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && shown) {
            e.stopPropagation();
            setShown(false);
          }
        }}
      >
        <span>
          Temporary<span className="temporary-more"> · deleted when you quit</span>
        </span>
      </Badge>
      {/* The chip's name already says this; the tooltip is for the eye. */}
      <span
        ref={tooltip}
        className="temporary-tooltip"
        id={tip}
        aria-hidden="true"
        style={place === null ? undefined : { left: place.left, maxWidth: place.maxWidth }}
      >
        {EXPLANATION}
      </span>
    </span>
  );
}
