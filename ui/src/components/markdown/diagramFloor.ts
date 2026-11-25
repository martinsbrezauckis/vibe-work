/**
 * The smallest width a Mermaid drawing may be shown at in the chat column. Wide drawings are scaled down to the
 * column only to this floor; below it they keep the floor size and their area scrolls sideways.
 *
 * The value reaches the page as two class names on our own wrapper, never as an inline style or inside the passive
 * markup (which refuses styles): CSS cannot read the drawing's own width, so the floor is rounded up to a step of a
 * 2^(1/16) ladder (at most 4.5 % above the exact floor) and `markdown-rich.css` defines one mantissa class and one
 * exponent class per ladder rung (`--diagram-m` times `--diagram-e`).
 */

/** A drawing is never shown below 3/4 of its natural size. */
const MIN_DRAWING_SCALE = 0.75;
/** Drawn text is never shown smaller than this many pixels (labels under 12 units raise the floor above 3/4). */
const MIN_TEXT_PX = 9;
/** Mantissas 2^(i/16) of the ladder; the same numbers are in markdown-rich.css (`.md-diagram-m0` to `-m15`). */
const MANTISSAS = [
  1, 1.0443, 1.0905, 1.1388, 1.1892, 1.2419, 1.2968, 1.3543, 1.4142, 1.4768, 1.5422, 1.6105, 1.6818,
  1.7563, 1.834, 1.9152,
];
/** Smallest rung in px (`.md-diagram-e0`); each exponent class doubles it up to 16384 (the drawing size limit). */
const BASE_PX = 256;
const EXPONENTS = 7;

const RUNGS = Array.from({ length: EXPONENTS * MANTISSAS.length }, (_, n) => {
  const exponent = Math.floor(n / MANTISSAS.length);
  const mantissa = n % MANTISSAS.length;
  return {
    px: Math.round(BASE_PX * 2 ** exponent * (MANTISSAS[mantissa] ?? 1)),
    className: `md-diagram-m${String(mantissa)} md-diagram-e${String(exponent)}`,
  };
});

/** Exact floor for a drawing: 3/4 of its width, raised so that its smallest label is drawn at 9 px or more. */
export function floorWidth(natural: number, smallestText: number | null): number {
  const scale =
    smallestText !== null && smallestText > 0
      ? Math.min(1, Math.max(MIN_DRAWING_SCALE, MIN_TEXT_PX / smallestText))
      : MIN_DRAWING_SCALE;
  return Math.round(natural * scale);
}

/** The ladder rung for a floor: the lowest rung at or above it, and never above the natural width. */
export function floorRung(
  floor: number,
  natural: number,
): { px: number; className: string } | null {
  let index = RUNGS.findIndex((rung) => rung.px >= floor);
  if (index < 0) index = RUNGS.length - 1;
  if ((RUNGS[index]?.px ?? 0) > natural) index -= 1;
  return RUNGS[index] ?? null;
}

/**
 * Class names for the floor of a drawing, read from our own renderer's output (never from the passive copy):
 * its root `width` and its smallest `font-size`. Null when the output carries no width.
 */
export function drawingFloorClasses(svg: string): string | null {
  const width = /^<svg\s[^>]*?\bwidth="(\d+(?:\.\d+)?)"/.exec(svg)?.[1];
  const natural = width === undefined ? NaN : Number(width);
  if (!Number.isFinite(natural) || natural <= 0) return null;
  const sizes = [...svg.matchAll(/\bfont-size="(\d+(?:\.\d+)?)"/g)].map((m) => Number(m[1]));
  const smallest = sizes.length > 0 ? Math.min(...sizes) : null;
  return floorRung(floorWidth(natural, smallest), natural)?.className ?? null;
}
