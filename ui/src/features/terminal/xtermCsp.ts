/**
 * Makes xterm's stylesheets work under our production CSP (`style-src 'self'`, no inline styles)
 * without relaxing it. xterm 6 creates three `<style>` elements at runtime (DOM renderer theme and
 * dimensions, viewport scrollbar) and adds `style` attributes with `setAttribute` for truecolor
 * cells; the CSP blocks all of them.
 *
 * How:
 * 1. Build time, `ui/vite.xterm-csp.ts`: in xterm's bundle, `createElement("style")` becomes
 *    `createElement("vw-xterm-style")`, and `_addStyle` sets `style.cssText` (CSSOM) instead of the
 *    `style` attribute. The plugin fails the build if xterm's code no longer matches exactly.
 * 2. Run time, this module: `vw-xterm-style` is a custom element that applies its text content
 *    through a constructable stylesheet adopted by its document (CSSOM: not an inline style, so
 *    `style-src` does not apply). Its own text is never rendered (`terminal.css` hides it).
 *
 * Depends on: the browser's custom elements and constructable stylesheets (WebView2, WebKitGTK
 * 2.40+, Safari 16.4+). Called by: session.ts (`installXtermCspShim` before xterm opens).
 * Invariants: one adopted sheet per element, removed when the element leaves the document; the
 * sheet always holds the element's current text.
 */

/** The tag the build-time rewrite gives xterm's style elements. */
export const XTERM_STYLE_TAG = "vw-xterm-style";

/** A style element whose rules are applied through CSSOM. */
class XtermStyle extends HTMLElement {
  #sheet: CSSStyleSheet | null = null;
  #root: Document | ShadowRoot | null = null;
  #observer: MutationObserver | null = null;

  connectedCallback(): void {
    const root = this.getRootNode();
    if (!(root instanceof Document) && !(root instanceof ShadowRoot)) return;
    const sheet = new CSSStyleSheet();
    root.adoptedStyleSheets = [...root.adoptedStyleSheets, sheet];
    this.#root = root;
    this.#sheet = sheet;
    this.#apply();
    this.#observer = new MutationObserver(() => {
      this.#apply();
    });
    this.#observer.observe(this, { childList: true, characterData: true, subtree: true });
  }

  disconnectedCallback(): void {
    this.#observer?.disconnect();
    this.#observer = null;
    const root = this.#root;
    const sheet = this.#sheet;
    if (root !== null && sheet !== null) {
      root.adoptedStyleSheets = root.adoptedStyleSheets.filter((s) => s !== sheet);
    }
    this.#root = null;
    this.#sheet = null;
  }

  #apply(): void {
    this.#sheet?.replaceSync(this.textContent);
  }
}

/** Defines the custom element once per window. */
export function installXtermCspShim(): void {
  if (customElements.get(XTERM_STYLE_TAG) === undefined) {
    customElements.define(XTERM_STYLE_TAG, XtermStyle);
  }
}
