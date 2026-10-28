/** Permission-card clipping checks for the ui/layout route. Uses the browser's rectangles and scroll extents.
 * Called by inspectRoutes for visible cards; intentional scrolling is allowed, unreadable payloads and controls
 * cut by a hidden frame are reported even when the body itself has collapsed to zero height.
 */
const CUT = /\b(hidden|clip)\b/;
const SCROLL = /\b(auto|scroll)\b/;

/** Names the parts of a visible card whose content cannot be read or reached. */
export function clippedCardParts(card: Element): string[] {
  const parts: string[] = [];
  for (const selector of [".ask-body", "pre.cmd", ".perms", ".choices button"]) {
    for (const el of card.querySelectorAll(selector)) {
      const style = getComputedStyle(el);
      if (style.display === "none" || style.visibility === "hidden") continue;
      const box = el.getBoundingClientRect();
      const line = parseFloat(style.lineHeight) || parseFloat(style.fontSize) || 16;
      let clipped = box.width < 1 || box.height < Math.min(line, el.scrollHeight);
      if (CUT.test(style.overflowY || style.overflow))
        clipped ||= el.scrollHeight > el.clientHeight + 1;
      if (CUT.test(style.overflowX || style.overflow))
        clipped ||= el.scrollWidth > el.clientWidth + 1;
      // A scrollable body or command can deliberately have more content than fits. Hidden card chrome cannot.
      for (let up = el.parentElement; up; up = up.parentElement) {
        const s = getComputedStyle(up);
        const r = up.getBoundingClientRect();
        if (CUT.test(s.overflowY || s.overflow))
          clipped ||= box.top < r.top - 1 || box.bottom > r.bottom + 1;
        if (CUT.test(s.overflowX || s.overflow))
          clipped ||= box.left < r.left - 1 || box.right > r.right + 1;
        // Controls partly hidden by their scroll frame need attention too; fully scrolled-away controls can
        // deliberately sit outside the viewport. Payload text retains its own intentional scrolling.
        if (selector === ".choices button" && SCROLL.test(s.overflowY || s.overflow)) {
          const top = r.top + up.clientTop;
          const bottom = top + up.clientHeight;
          if (box.bottom > top && box.top < bottom)
            clipped ||= box.top < top - 1 || box.bottom > bottom + 1;
        }
        // The transcript can scroll a whole card; an inner scroll can reveal a long payload.
        if (up === card || SCROLL.test(`${s.overflow} ${s.overflowY} ${s.overflowX}`)) break;
      }
      if (clipped) parts.push(selector);
    }
  }
  return [...new Set(parts)];
}
