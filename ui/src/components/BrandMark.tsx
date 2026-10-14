/**
 * The Vibe Work mark: two pixel "V" steps in the brand oranges. Decorative.
 * Depends on: react, styles/shell.css (`.brand-mark .c1|c2|c3` colours). Called by: the title bar.
 */
import type { ReactElement } from "react";

const CELLS: readonly (readonly [number, number, "c1" | "c2" | "c3"])[] = [
  [0, 1, "c1"],
  [1, 3, "c2"],
  [2, 5, "c2"],
  [3, 7, "c3"],
  [4, 5, "c2"],
  [5, 3, "c2"],
  [6, 1, "c1"],
  [7, 3, "c2"],
  [8, 5, "c2"],
  [9, 7, "c3"],
  [10, 5, "c2"],
  [11, 3, "c2"],
  [12, 1, "c1"],
];

/** The brand mark at 26x19 px. */
export function BrandMark(): ReactElement {
  return (
    <svg
      className="brand-mark"
      width="26"
      height="19"
      viewBox="0 0 14 10"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      {CELLS.map(([x, y, tone]) => (
        <rect key={x} className={tone} x={x} y={y} width="2" height="2" />
      ))}
    </svg>
  );
}
