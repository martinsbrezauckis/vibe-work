/**
 * The `ui/*` routes for evidence and layout checks: `revision` and `settle` (the mutation counter a
 * driver waits on before a screenshot, ShellX `debug_api.rs:443-452` @ e38c2b5c `uiRevision`), `rect` (an
 * element's rectangle, used by `/screenshot?crop=`) and `layout` (cheap automatic checks of what is shown: menus,
 * popovers and dialogs inside the window, no horizontal scroll, no cut-off button/heading text or card content).
 * The Rust side
 * lists the same names (`crates/vw-host/src/host.rs` SHELL_ROUTES). `secrets` reports where secret fields are, so
 * window screenshots blank them; `rect` refuses selectors that test a value (`secretSafety.ts`).
 * Depends on: state/uiControl, AppContext types. Called by: useShellWiring.
 * Invariants: read-only (nothing here changes the UI); every body is validated (`invalidParams`); `settle` always
 * answers within its timeout (default 5 s, at most 30 s) and says whether the UI really went quiet.
 */
import { UiControlError } from "../../state/uiControl";
import { queryNeutral } from "./secretSafety";
import { screenshotSecrets } from "./screenshotSecrets";
import type { App } from "./AppContext";
import { elementVisible } from "./elementVisibility";
import { clippedCardParts } from "./inspectCardLayout";

/** Overlay-like elements the layout check looks at. */
const OVERLAYS =
  '[role="menu"], [role="listbox"], [role="dialog"], [role="tooltip"], .popover, .menu';
/** Elements whose text must not be cut off. */
const TEXTUAL = "button, h1, h2, h3, [role='tab'], [role='menuitem']";

function asObject(body: unknown): Record<string, unknown> {
  if (body === undefined || body === null) return {};
  if (typeof body === "object" && !Array.isArray(body)) return body as Record<string, unknown>;
  throw new UiControlError("invalidParams", "the body must be a JSON object");
}

function numberIn(value: unknown, fallback: number, max: number): number {
  if (value === undefined) return fallback;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0)
    throw new UiControlError("invalidParams", "a duration must be a non-negative number of ms");
  return Math.min(value, max);
}

/** A short, stable name for an element in reports (role or tag, then its label or class). */
function nameOf(el: Element): string {
  const role = el.getAttribute("role") ?? el.tagName.toLowerCase();
  const label = el.getAttribute("aria-label") ?? el.getAttribute("data-testid") ?? "";
  const cls = typeof el.className === "string" ? el.className.split(" ")[0] : "";
  return [role, label || cls].filter(Boolean).join(":");
}

function shown(el: Element): boolean {
  const r = el.getBoundingClientRect();
  if (r.width < 1 || r.height < 1) return false;
  const style = getComputedStyle(el);
  return style.visibility !== "hidden" && style.display !== "none" && style.opacity !== "0";
}

/** Overflow values that clip an element's content. */
const CLIPPING = /\b(hidden|clip|scroll|auto)\b/;

/** A row can overflow in measurement because its title deliberately ellipsizes inside the button. */
function rowTitleEllipsizes(el: Element): boolean {
  const titleClass = el.classList.contains("srow-main")
    ? "t"
    : el.classList.contains("proj-h")
      ? "pname"
      : null;
  if (titleClass === null) return false;
  const title = [...el.children].find((child) => child.classList.contains(titleClass));
  if (title === undefined) return false;
  const style = getComputedStyle(title);
  if (style.textOverflow !== "ellipsis" || !CLIPPING.test(`${style.overflowX} ${style.overflow}`))
    return false;
  const box = el.getBoundingClientRect();
  const inside = (child: Element) => {
    const rect = child.getBoundingClientRect();
    return rect.left >= box.left - 1 && rect.right <= box.right + 1;
  };
  return (
    [...el.children].every(inside) &&
    [...el.childNodes].every(
      (node) => node.nodeType !== Node.TEXT_NODE || node.textContent?.trim() === "",
    )
  );
}

/** Ancestors whose overflow can affect this box, respecting fixed-position containing blocks. */
function* clippingAncestors(el: Element) {
  let fixed = getComputedStyle(el).position === "fixed";
  for (let up = el.parentElement; up !== null; up = up.parentElement) {
    const style = getComputedStyle(up);
    // A viewport-fixed modal escapes ordinary overflow ancestors. A transform/filter/containment
    // instead makes that ancestor its containing block, so clipping above it still matters.
    const containsFixed =
      [style.transform, style.perspective, style.filter, style.backdropFilter].some(
        (value) => value && value !== "none",
      ) ||
      /layout|paint|strict|content/.test(style.contain) ||
      /transform|perspective|filter/.test(style.willChange) ||
      style.contentVisibility === "auto";
    if (containsFixed) fixed = false;
    if (!fixed) yield { up, style };
    fixed ||= style.position === "fixed";
  }
}

/**
 * True when a clipping ancestor leaves nothing showing, such as a collapsed column or closed drawer.
 * Its own text overflow is then not something a person can see.
 */
function isClippedAway(el: Element): boolean {
  const box = el.getBoundingClientRect();
  for (const { up, style } of clippingAncestors(el)) {
    if (!CLIPPING.test(`${style.overflow} ${style.overflowX} ${style.overflowY}`)) continue;
    const clip = up.getBoundingClientRect();
    const overlapX = Math.min(box.right, clip.right) - Math.max(box.left, clip.left);
    const overlapY = Math.min(box.bottom, clip.bottom) - Math.max(box.top, clip.top);
    if (overlapX < 1 || overlapY < 1) return true;
  }
  return false;
}

/** Visible controls cut horizontally by an ancestor still need reporting when their own text fits. */
function ancestorCuts(el: Element): boolean {
  const box = el.getBoundingClientRect();
  let cut = false;
  for (const { up, style } of clippingAncestors(el)) {
    if (style.visibility === "hidden" || style.display === "none") return false;
    const clip = up.getBoundingClientRect();
    if (!CLIPPING.test(`${style.overflow} ${style.overflowX} ${style.overflowY}`)) continue;
    // Folded columns and rows scrolled outside the vertical viewport are intentionally absent.
    if (clip.width < 1 || clip.height < 1 || box.bottom <= clip.top || box.top >= clip.bottom)
      return false;
    if (CLIPPING.test(`${style.overflowX} ${style.overflow}`))
      cut ||= box.left < clip.left - 1 || box.right > clip.right + 1;
  }
  return cut;
}

/**
 * The "text cut off" decision for one shown element (module docs): its content is wider than its box, it is not a
 * 1-px visually hidden element, it does not end in a designed ellipsis, and it is really on screen.
 */
export function isTextCutOff(el: Element): boolean {
  // A visually hidden element (clipped to 1 px, like a screen-reader-only title) is cut on purpose.
  const box = el.getBoundingClientRect();
  if (box.width <= 1.5 && box.height <= 1.5) return false;
  if (ancestorCuts(el)) return true;
  const style = getComputedStyle(el);
  // An ellipsis is a designed cut, not an accident.
  if (style.textOverflow === "ellipsis" || el.scrollWidth <= el.clientWidth + 1) return false;
  if (rowTitleEllipsizes(el)) return false;
  return !isClippedAway(el);
}

/** Counts DOM mutations: the counter a driver reads before an action and waits on after it. */
function createRevision() {
  let revision = 0;
  let lastAt = performance.now();
  const observer = new MutationObserver((records) => {
    revision += records.length > 0 ? 1 : 0;
    lastAt = performance.now();
  });
  observer.observe(document.documentElement, {
    subtree: true,
    childList: true,
    attributes: true,
    characterData: true,
  });
  const running = () =>
    typeof document.getAnimations === "function"
      ? document.getAnimations().filter((a) => a.playState === "running").length
      : 0;
  return {
    read: () => ({
      revision,
      quietForMs: Math.round(performance.now() - lastAt),
      animations: running(),
    }),
    async settle(quietMs: number, timeoutMs: number) {
      const started = performance.now();
      for (;;) {
        // Two frames: what a mutation scheduled is painted before it counts as quiet.
        await new Promise<void>((resolve) => {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              resolve();
            });
          });
        });
        const quiet = performance.now() - lastAt >= quietMs && running() === 0;
        const waited = performance.now() - started;
        if (quiet || waited >= timeoutMs)
          return { revision, settled: quiet, waitedMs: Math.round(waited) };
        await new Promise((resolve) => setTimeout(resolve, Math.min(50, quietMs)));
      }
    },
    stop: () => {
      observer.disconnect();
    },
  };
}

/** The layout check (module docs). */
function layoutReport() {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const overlays = [...document.querySelectorAll(OVERLAYS)].filter(shown).map((el) => {
    const r = el.getBoundingClientRect();
    const inside = r.left >= -0.5 && r.top >= -0.5 && r.right <= vw + 0.5 && r.bottom <= vh + 0.5;
    return {
      name: nameOf(el),
      inside,
      rect: { x: r.left, y: r.top, width: r.width, height: r.height },
    };
  });
  const cutOff = [...document.querySelectorAll(TEXTUAL)]
    .filter(shown)
    .filter(isTextCutOff)
    .slice(0, 20)
    .map(nameOf);
  const root = document.documentElement;
  const horizontalScroll = root.scrollWidth > vw + 1;
  const cards = [...document.querySelectorAll(".approval")]
    .filter((card) => shown(card) && !isClippedAway(card))
    .flatMap((card) => {
      const parts = clippedCardParts(card);
      return parts.length === 0
        ? []
        : [`card content clipped: ${nameOf(card)} (${parts.join(", ")})`];
    });
  const problems = [
    ...overlays.filter((o) => !o.inside).map((o) => `outside the window: ${o.name}`),
    ...cutOff.map((n) => `text cut off: ${n}`),
    ...cards,
    ...(horizontalScroll ? [`horizontal scroll: ${String(root.scrollWidth)} > ${String(vw)}`] : []),
  ];
  return {
    viewport: { width: vw, height: vh, dpr: window.devicePixelRatio },
    overlays,
    cutOff,
    horizontalScroll,
    problems,
  };
}

/** Registers `revision`, `settle`, `rect` and `layout`; returns the function that removes them. */
export function registerInspectRoutes(app: App): () => void {
  const counter = createRevision();
  const off = [
    app.uiControl.register("revision", () => counter.read()),
    app.uiControl.register("settle", (body) => {
      const { quietMs, timeoutMs } = asObject(body);
      return counter.settle(numberIn(quietMs, 250, 10_000), numberIn(timeoutMs, 5_000, 30_000));
    }),
    app.uiControl.register("rect", (body) => {
      const { selector } = asObject(body);
      if (typeof selector !== "string" || selector === "")
        throw new UiControlError("invalidParams", "selector must be a non-empty CSS selector");
      const base = { viewportWidth: window.innerWidth, viewportHeight: window.innerHeight };
      // Decided on the result with every secret value emptied (`secretSafety.ts`): no value oracle.
      const found = queryNeutral(() => {
        let el: Element | null;
        try {
          el = document.querySelector(selector);
        } catch {
          throw new UiControlError("invalidParams", "that is not a valid CSS selector");
        }
        return el === null || !elementVisible(el) ? null : el.getBoundingClientRect();
      });
      if (found === null) return { found: false, ...base };
      return {
        found: true,
        x: found.left,
        y: found.top,
        width: found.width,
        height: found.height,
        ...base,
      };
    }),
    app.uiControl.register("layout", () => layoutReport()),
    // where the secret fields are, so a window screenshot can blank them (`secretSafety.ts`).
    // `{lock: true}` before a capture (secret fields show dots, Show is disabled), `{lock: false}` after it; the
    // Debug API compares both answers and discards the capture when anything moved (`secretSafety.ts`).
    app.uiControl.register(
      "secrets",
      (body) => screenshotSecrets(asObject(body).lock),
      () => true,
    ),
  ];
  return () => {
    counter.stop();
    for (const remove of off) remove();
  };
}
