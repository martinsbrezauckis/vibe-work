/**
 * An empty state (feel item 13): one pixel picture, one sentence, and at most one action. "An empty screen is
 * an invitation to act": the sentence says what to do next.
 * Depends on: PixelGlyph. Called by: the session list, the Projects overview, the Changes tab.
 */
import type { ReactElement, ReactNode } from "react";

import { PixelGlyph, type PixelName } from "./PixelGlyph";

/** The block. `small` (32 px picture) suits a list column; the default (48 px) a page. */
export function EmptyState({
  glyph,
  children,
  small = false,
  className = "",
}: {
  glyph: PixelName;
  children: ReactNode;
  small?: boolean;
  className?: string;
}): ReactElement {
  return (
    <div className={`empty-state${small ? " small" : ""} ${className}`.trim()}>
      <PixelGlyph name={glyph} size={small ? 32 : 48} />
      <div className="empty-text">{children}</div>
    </div>
  );
}
