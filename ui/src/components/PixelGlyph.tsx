/**
 * Pixel pictures for empty states (one of the four "pixel touches" of our own): a folder, a speech bubble
 * and a checked box drawn on a 16x16 grid of squares, in `currentColor`. Drawn for Vibe Work; not Mistral's assets.
 * Depends on: react. Called by: EmptyState.
 */
import type { ReactElement } from "react";

/** The pictures, one string per row (`#` = a filled pixel). */
const ART = {
  folder: [
    "................",
    "................",
    "..#####.........",
    ".#.....#........",
    ".#......#######.",
    ".#.............#",
    ".###############",
    ".#.............#",
    ".#.............#",
    ".#.............#",
    ".#.............#",
    ".#.............#",
    ".###############",
  ],
  chat: [
    "................",
    "..###########...",
    ".#...........#..",
    ".#...........#..",
    ".#..##.##.##.#..",
    ".#...........#..",
    ".#...........#..",
    "..#####..####...",
    "......#.#.......",
    "......##........",
  ],
  check: [
    "................",
    ".##############.",
    ".#............#.",
    ".#............#.",
    ".#.........##.#.",
    ".#........##..#.",
    ".#.......##...#.",
    ".#.##...##....#.",
    ".#..##.##.....#.",
    ".#...###......#.",
    ".#....#.......#.",
    ".#............#.",
    ".#............#.",
    ".##############.",
  ],
} as const;

/** Picture names. */
export type PixelName = keyof typeof ART;

/** One picture at `size` CSS pixels (a multiple of 16 keeps every pixel crisp). */
export function PixelGlyph({ name, size = 48 }: { name: PixelName; size?: number }): ReactElement {
  const rects: ReactElement[] = [];
  ART[name].forEach((row, y) => {
    for (let x = 0; x < row.length; x++)
      if (row[x] === "#")
        rects.push(<rect key={`${String(x)}-${String(y)}`} x={x} y={y} width="1" height="1" />);
  });
  return (
    <svg
      className="pixel"
      width={size}
      height={size}
      viewBox="0 0 16 16"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      {rects}
    </svg>
  );
}
