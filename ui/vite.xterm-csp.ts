/**
 * Build-time rewrite of xterm's bundle so it never needs inline styles (the runtime half
 * and the reasons are in src/features/terminal/xtermCsp.ts).
 *
 * Role: a Vite plugin applied to `@xterm/xterm/lib/xterm.mjs` in both dev and build (the package is
 * excluded from dependency pre-bundling in vite.config.ts so dev and production run the same code).
 * Each rewrite must match exactly the expected number of times; otherwise the build fails with a
 * message to re-check the shim, so an xterm upgrade can never silently bring inline styles back.
 * Dependencies: vite types. Callers: vite.config.ts.
 */
import type { Plugin } from "vite";

/** The custom element `features/terminal/xtermCsp.ts` defines. */
const STYLE_TAG = "vw-xterm-style";
const XTERM_ENTRY = /[\\/]@xterm[\\/]xterm[\\/]lib[\\/]xterm\.mjs$/;

interface Rewrite {
  name: string;
  find: RegExp;
  replace: string;
  count: number;
}

const REWRITES: readonly Rewrite[] = [
  {
    // DomRenderer theme + dimensions, Viewport scrollbar colours (xterm 6.0.0).
    name: "style elements",
    find: /\.createElement\("style"\)/g,
    replace: `.createElement("${STYLE_TAG}")`,
    count: 3,
  },
  {
    // DomRendererRowFactory._addStyle: truecolor cells (`style` attribute -> CSSOM).
    name: "row style attribute",
    find: /_addStyle\((\w+),(\w+)\)\{\1\.setAttribute\("style",`\$\{\1\.getAttribute\("style"\)\|\|""\}\$\{\2\};`\)\}/g,
    replace: '_addStyle($1,$2){$1.style.cssText+=$2+";"}',
    count: 1,
  },
];

/** Rewrites `code` (xterm's ESM bundle); throws when xterm no longer matches. Exported for tests. */
export function rewriteXterm(code: string): string {
  let out = code;
  for (const rewrite of REWRITES) {
    const found = out.match(rewrite.find)?.length ?? 0;
    if (found !== rewrite.count) {
      throw new Error(
        `xterm CSP rewrite "${rewrite.name}": expected ${String(rewrite.count)} match(es), found ${String(found)}. ` +
          "xterm changed: re-check src/features/terminal/xtermCsp.ts before upgrading.",
      );
    }
    out = out.replace(rewrite.find, rewrite.replace);
  }
  return out;
}

/** The Vite plugin. */
export function xtermCsp(): Plugin {
  return {
    name: "vw-xterm-csp",
    enforce: "pre",
    transform(code, id) {
      const path = id.split("?")[0] ?? id;
      if (!XTERM_ENTRY.test(path)) return null;
      return { code: rewriteXterm(code), map: null };
    },
  };
}
