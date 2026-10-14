/**
 * Terminal look from the design tokens: xterm needs concrete colour strings (its renderers do colour
 * maths and cannot resolve CSS variables), so the
 * `--term-*` tokens of the current theme are read with `getComputedStyle`.
 * Depends on: styles/tokens.css (`--term-*`, `--mono`). Called by: session.ts (at mount and on theme
 * change).
 * Invariants: every value comes from a token (no literal colours here, `check_colors.mjs`); a missing
 * token leaves xterm's own default for that slot.
 */
import type { ITheme } from "@xterm/xterm";

/** xterm theme slot -> token name. */
const SLOTS: ReadonlyArray<readonly [keyof ITheme, string]> = [
  ["background", "--term-bg"],
  ["foreground", "--term-fg"],
  ["cursor", "--term-cursor"],
  ["cursorAccent", "--term-bg"],
  ["selectionBackground", "--term-selection"],
  ["black", "--term-black"],
  ["red", "--term-red"],
  ["green", "--term-green"],
  ["yellow", "--term-yellow"],
  ["blue", "--term-blue"],
  ["magenta", "--term-magenta"],
  ["cyan", "--term-cyan"],
  ["white", "--term-white"],
  ["brightBlack", "--term-bright-black"],
  ["brightRed", "--term-bright-red"],
  ["brightGreen", "--term-bright-green"],
  ["brightYellow", "--term-bright-yellow"],
  ["brightBlue", "--term-bright-blue"],
  ["brightMagenta", "--term-bright-magenta"],
  ["brightCyan", "--term-bright-cyan"],
  ["brightWhite", "--term-bright-white"],
];

/** Font size in px when the token cannot be read (tests, no stylesheet). */
const FALLBACK_FONT_SIZE = 13;

/** The terminal's colours, font family and size for the theme now applied to `<html>`. */
export function terminalLook(root: HTMLElement = document.documentElement): {
  theme: ITheme;
  fontFamily: string;
  fontSize: number;
} {
  const style = getComputedStyle(root);
  const theme: Record<string, string> = {};
  for (const [slot, token] of SLOTS) {
    const value = style.getPropertyValue(token).trim();
    if (value !== "") theme[slot] = value;
  }
  const size = Number.parseFloat(style.getPropertyValue("--term-font-size"));
  const family = style.getPropertyValue("--mono").trim();
  return {
    theme,
    fontFamily: family === "" ? "monospace" : family,
    fontSize: Number.isFinite(size) && size > 0 ? size : FALLBACK_FONT_SIZE,
  };
}
