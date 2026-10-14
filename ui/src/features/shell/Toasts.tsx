/**
 * The toast stack. Placement keeps every visible control, heading, protected region and native
 * view clear. If no free area fits, one navigable notice uses the existing status bar until empty.
 * Floating stacks clip animation travel to their measured padded bounds.
 * Information stays long enough to read; warnings and errors remain until dismissed. Each toast keeps its own time (a hairline shows it; hovering pauses it), slides out when
 * it goes, and the others glide into place.
 * Native views (`[data-toast-avoid="native"]`) paint above the page, so they are also protected.
 * Depends on: AppContext, state/createStore, toastPlacement, components/Icon, components/nativeLayer. Called by: Shell.
 * Invariants: the stack is a polite live region; moving focus into a field, or a native view appearing, moving or
 * going, re-places the stack; so does a region to keep clear changing size after a layout switch (at 390 px
 * the narrow layout settled after the `resize` event, and the stack stayed in a corner over the composer's Send); the
 * floating placement never reserves native view space; an opened notification popover reserves its bounds.
 */
import { type CSSProperties, useEffect, useLayoutEffect, useRef, useState } from "react";

import { ToastItem } from "./ToastItem";
import { NotificationsPanel } from "./NotificationsPanel";
import { IconButton } from "../../components/Controls";
import { NATIVE_LAYOUT_EVENT, nativeRects, reserve } from "../../components/nativeLayer";
import { useStore } from "../../state/createStore";
import { useApp } from "./AppContext";
import { BOTTOM, type Corner, type Rect, placeToastsOver } from "./toastPlacement";

const PROTECTED =
  '[data-toast-avoid]:not([data-toast-avoid="native"]), input, textarea, select, button, a[href], label, summary, [contenteditable]:not([contenteditable="false"]), [role="button"], [role="link"], [role="tab"], [role="menuitem"], [role="checkbox"], [role="radio"], [role="switch"], [role="textbox"], [tabindex], h1, h2, h3, h4, h5, h6';

function rectOf(element: Element): Rect {
  const r = element.getBoundingClientRect();
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
}

function visibleRect(element: Element): Rect | null {
  const rect = rectOf(element);
  for (let parent = element.parentElement; parent !== null; parent = parent.parentElement) {
    const style = window.getComputedStyle(parent);
    const clipsX =
      /^(auto|scroll|hidden|clip)$/.test(style.overflowX) ||
      /^(auto|scroll|hidden|clip)$/.test(style.overflow);
    const clipsY =
      /^(auto|scroll|hidden|clip)$/.test(style.overflowY) ||
      /^(auto|scroll|hidden|clip)$/.test(style.overflow);
    if (!clipsX && !clipsY) continue;
    const bounds = rectOf(parent);
    if (clipsX) {
      rect.left = Math.max(rect.left, bounds.left);
      rect.right = Math.min(rect.right, bounds.right);
    }
    if (clipsY) {
      rect.top = Math.max(rect.top, bounds.top);
      rect.bottom = Math.min(rect.bottom, bounds.bottom);
    }
  }
  return rect.right > rect.left && rect.bottom > rect.top ? rect : null;
}

/** The reservation key of the stack in `nativeLayer`. */
const RESERVATION = "toasts";

/** Visible rectangles that must stay uncovered right now (native views apart: `nativeRects`). */
function rectsToAvoid(): Rect[] {
  return Array.from(document.querySelectorAll(PROTECTED)).flatMap((element) => {
    if (element.closest(".toasts, .toast-dialog, [hidden], [inert], [aria-hidden='true']"))
      return [];
    const style = window.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return [];
    const rect = visibleRect(element);
    return rect === null ? [] : [rect];
  });
}

function nativeToAvoid(): Rect[] {
  return nativeRects()
    .filter((rect) => rect.width > 0 && rect.height > 0)
    .map((r) => ({ left: r.left, top: r.top, right: r.right, bottom: r.bottom }));
}

/** A comparable text of the regions to keep clear (placement re-runs only when it changes). */
function signature(rects: readonly Rect[]): string {
  return rects.map((r) => [r.left, r.top, r.right, r.bottom].map(Math.round).join(",")).join(";");
}

/** The toast stack. */
export function Toasts() {
  const { toasts } = useApp();
  const list = useStore(toasts.store, (s) => s);
  const reading = useStore(toasts.panel, (s) => s.open);
  const ref = useRef<HTMLDivElement>(null);
  const [corner, setCorner] = useState<Corner>("bottom-right");
  const [lift, setLift] = useState(0);
  const [leftInset, setLeftInset] = useState(18);
  const [docked, setDocked] = useState(false);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const dockedNow = useRef(false);
  const [tick, setTick] = useState(0);
  /** The regions the last placement kept clear. */
  const placedFor = useRef("");
  const measured = () => {
    const stack = ref.current?.getBoundingClientRect();
    const stackSize =
      stack === undefined
        ? ""
        : `${String(Math.round(stack.width))},${String(Math.round(stack.height))}`;
    const native = nativeToAvoid();
    return `${signature(rectsToAvoid())}|${signature(native)}|${stackSize}`;
  };

  useEffect(() => {
    const bump = () => {
      setTick((n) => n + 1);
    };
    const events = ["focusin", "focusout", "resize", "scroll", NATIVE_LAYOUT_EVENT];
    for (const type of events) window.addEventListener(type, bump, true);
    return () => {
      for (const type of events) window.removeEventListener(type, bump, true);
      reserve(RESERVATION, null);
    };
  }, []);

  useLayoutEffect(() => {
    const stack = ref.current;
    if (stack === null || list.length === 0) {
      dockedNow.current = false;
      setDocked(false);
      reserve(RESERVATION, null);
      return;
    }
    placedFor.current = measured();
    if (dockedNow.current) {
      reserve(RESERVATION, null);
      return;
    }
    const rail = document.querySelector('[data-toast-avoid="rail"]')?.getBoundingClientRect();
    const visibleRail = document.querySelector('[data-toast-avoid="rail"]');
    const inset =
      rail !== undefined &&
      rail.width > 0 &&
      visibleRail !== null &&
      window.getComputedStyle(visibleRail).visibility !== "hidden"
        ? Math.max(18, rail.right + 8)
        : 18;
    const size = stack.getBoundingClientRect();
    const viewport = { width: window.innerWidth, height: window.innerHeight, leftInset: inset };
    const stackSize = { width: size.width, height: size.height };
    const native = nativeToAvoid();
    const avoid = rectsToAvoid();
    const placement = placeToastsOver(viewport, stackSize, avoid, native);
    if (placement.blocked) {
      dockedNow.current = true;
      setDocked(true);
      reserve(RESERVATION, null);
      return;
    }
    setCorner(placement.corner);
    setLift(placement.lift);
    setLeftInset(inset);
    reserve(RESERVATION, null);
  }, [list, tick]);

  // while toasts show, a region to keep clear that changes size (the layout switching after a resize, the
  // composer growing) places the stack again. Only a real change bumps: the observer's first report after
  // `observe` finds the same regions, so this cannot loop.
  const showing = list.length > 0;
  useEffect(() => {
    if (!showing) return;
    const bumpIfChanged = () => {
      const next = measured();
      if (next !== placedFor.current) {
        placedFor.current = next;
        setTick((n) => n + 1);
      }
    };
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(bumpIfChanged);
    for (const element of Array.from(document.querySelectorAll(PROTECTED)))
      observer?.observe(element);
    if (ref.current !== null) observer?.observe(ref.current);
    observer?.observe(document.documentElement);
    const mutations = new MutationObserver((records) => {
      if (records.some((record) => record.type === "childList")) {
        for (const element of Array.from(document.querySelectorAll(PROTECTED)))
          observer?.observe(element);
      }
      bumpIfChanged();
    });
    mutations.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["hidden", "inert", "aria-hidden", "style", "class", "tabindex", "role"],
    });
    return () => {
      observer?.disconnect();
      mutations.disconnect();
    };
  }, [showing, tick]);

  // when a toast leaves, the ones that stay glide to their new places instead of jumping (FLIP).
  const tops = useRef(new Map<string, number>());
  useLayoutEffect(() => {
    const stack = ref.current;
    if (stack === null) return;
    if (docked) {
      // A prior floating FLIP must not travel out of the compact footer after docking.
      for (const element of Array.from(stack.querySelectorAll<HTMLElement>("[data-toast-id]"))) {
        if (typeof element.getAnimations === "function") {
          for (const animation of element.getAnimations()) animation.cancel();
        }
      }
      tops.current.clear();
      return;
    }
    const now = new Map<string, number>();
    for (const el of Array.from(stack.querySelectorAll<HTMLElement>("[data-toast-id]"))) {
      const id = el.dataset["toastId"] ?? "";
      const top = el.getBoundingClientRect().top;
      now.set(id, top);
      const before = tops.current.get(id);
      if (
        before !== undefined &&
        before !== top &&
        !reducedMotion() &&
        typeof el.animate === "function"
      )
        el.animate(
          [{ transform: `translateY(${String(before - top)}px)` }, { transform: "none" }],
          {
            duration: 240,
            easing: "cubic-bezier(.16, 1, .3, 1)",
          },
        );
    }
    tops.current = now;
  }, [list, corner, lift, docked]);

  const firstError = list.findIndex((toast) => toast.kind === "error");
  const fallbackIndex = firstError >= 0 ? firstError : list.length - 1;
  const selectedIndex = list.findIndex((toast) => toast.id === selectedId);
  const activeIndex = selectedIndex >= 0 ? selectedIndex : fallbackIndex;
  const active = list[activeIndex];

  return (
    <>
      <NotificationsPanel />
      {list.length > 0 && (
        <div
          className="toasts"
          ref={ref}
          data-corner={corner}
          data-docked={docked ? "true" : undefined}
          aria-live="polite"
          style={
            {
              ...(!docked && lift > 0 ? { bottom: `${String(BOTTOM + lift)}px` } : {}),
              ...(!docked && corner.endsWith("left") ? { left: `${String(leftInset)}px` } : {}),
              "--toast-left-inset": `${String(leftInset)}px`,
            } as CSSProperties
          }
        >
          {docked && list.length > 1 && (
            <IconButton
              icon="arrow-left"
              aria-label="Previous notification"
              onClick={() => {
                setSelectedId(list[(activeIndex + list.length - 1) % list.length]?.id ?? null);
              }}
            />
          )}
          {list.map((toast) => (
            <ToastItem
              key={toast.id}
              toast={toast}
              docked={docked}
              active={docked && toast.id === active?.id}
              reading={reading}
              onOpen={() => {
                toasts.open(toast.id);
              }}
              onGone={() => {
                toasts.dismiss(toast.id);
              }}
            />
          ))}
          {docked && list.length > 1 && (
            <IconButton
              icon="chevron-right"
              aria-label="Next notification"
              onClick={() => {
                setSelectedId(list[(activeIndex + 1) % list.length]?.id ?? null);
              }}
            />
          )}
        </div>
      )}
    </>
  );
}

function reducedMotion(): boolean {
  return (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}
