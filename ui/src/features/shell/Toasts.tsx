/**
 * The toast stack. Placement keeps every visible control, heading, protected region and native
 * view clear. If no free area fits, one navigable notice uses the existing status bar until empty.
 * Floating stacks clip animation travel to their measured padded bounds.
 * Information stays long enough to read; warnings and errors remain until dismissed. Each toast keeps its own time (a hairline shows it; hovering pauses it), slides out when
 * it goes, and the others glide into place.
 * Native views (`[data-toast-avoid="native"]`) paint above the page, so they are also protected.
 * Depends on: AppContext, state/createStore, toastPlacement, components/Icon, components/nativeLayer. Called by: Shell.
 * Invariants: the stack is not a live region (the toast store says each toast once through the app's announcer);
 * a native view appearing, moving or going, or the window resizing, re-places the stack. Between those, the stack
 * keeps its corner and moves only when it now covers something it was placed clear of: a focused field, or a
 * region to keep clear that appeared or changed size (at 390 px the narrow layout settled after the `resize`
 * event, and the stack stayed in a corner over the composer's Send). Content the
 * user scrolled under the stack does not count, including rows a long conversation mounts while it scrolls. The
 * floating placement never reserves native view space; an opened notification popover reserves its bounds.
 * The conversation (`role="log"`) is content, not a control: it is focusable only so the keyboard can scroll it,
 * and the stack may sit over it right above the message box. Its rows' own buttons stay protected. Without this,
 * at 390 px a toast shown as a session's first message appeared ("The session started…") had no free spot and
 * became a bare "Notice" in the status bar.
 */
import { type CSSProperties, useEffect, useLayoutEffect, useRef, useState } from "react";

import { ToastItem } from "./ToastItem";
import { NotificationsPanel } from "./NotificationsPanel";
import { IconButton } from "../../components/Controls";
import { NATIVE_LAYOUT_EVENT, nativeRects, reserve } from "../../components/nativeLayer";
import { useStore } from "../../state/createStore";
import { useApp } from "./AppContext";
import {
  BOTTOM,
  type Corner,
  type Rect,
  coversAny,
  placeToastsOver,
  stackRect,
} from "./toastPlacement";

const PROTECTED =
  '.walkthrough-caption, [data-toast-avoid]:not([data-toast-avoid="native"]), input, textarea, select, button, a[href], label, summary, [contenteditable]:not([contenteditable="false"]), [role="button"], [role="link"], [role="tab"], [role="menuitem"], [role="checkbox"], [role="radio"], [role="switch"], [role="textbox"], [tabindex]:not([role="log"]), h1, h2, h3, h4, h5, h6';

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

/** A visible element to keep clear and the part of it that shows. */
interface Region {
  element: Element;
  rect: Rect;
}

/** Visible elements that must stay uncovered right now (native views apart: `nativeRects`). */
function regionsToAvoid(): Region[] {
  return Array.from(document.querySelectorAll(PROTECTED)).flatMap((element) => {
    if (
      !element.matches(".walkthrough-caption") &&
      element.closest(".toasts, .toast-dialog, [hidden], [inert], [aria-hidden='true']")
    )
      return [];
    const style = window.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return [];
    const rect = visibleRect(element);
    return rect === null ? [] : [{ element, rect }];
  });
}

/** A stable number per element, so a signature can tell the same region from a new one. */
const identities = new WeakMap<Element, number>();
let nextIdentity = 0;
function identityOf(element: Element): number {
  let id = identities.get(element);
  if (id === undefined) {
    nextIdentity += 1;
    id = nextIdentity;
    identities.set(element, id);
  }
  return id;
}

/**
 * Which regions to keep clear exist and how big they are. Positions are left out: scrolling moves regions without
 * changing the layout, and must not re-place the stack.
 */
function regionSignature(regions: readonly Region[]): string {
  return regions
    .map(({ element, rect }) =>
      [identityOf(element), rect.right - rect.left, rect.bottom - rect.top]
        .map(Math.round)
        .join(","),
    )
    .join(";");
}

/** Whether `element` sits in a container the user scrolled (since the set was started). */
function inScrolled(element: Element, scrolled: WeakSet<Element>): boolean {
  if (scrolled.has(document.documentElement)) return true;
  for (let parent = element.parentElement; parent !== null; parent = parent.parentElement)
    if (scrolled.has(parent)) return true;
  return false;
}

function nativeToAvoid(): Rect[] {
  return nativeRects()
    .filter((rect) => rect.width > 0 && rect.height > 0)
    .map((r) => ({ left: r.left, top: r.top, right: r.right, bottom: r.bottom }));
}

/** A comparable text of rectangles, positions included (native views move only with a layout event). */
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
  /** The regions the last check saw (which ones, their sizes, the native views and the stack's size). */
  const placedFor = useRef("");
  /** Where the floating stack was last placed (null: not floating). */
  const placed = useRef<{ corner: Corner; lift: number; inset: number } | null>(null);
  /** Containers scrolled since the last placement: what they hold moved under the stack by scrolling. */
  const scrolled = useRef(new WeakSet<Element>());
  const measured = () => {
    const stack = ref.current?.getBoundingClientRect();
    const stackSize =
      stack === undefined
        ? ""
        : `${String(Math.round(stack.width))},${String(Math.round(stack.height))}`;
    const native = nativeToAvoid();
    return `${regionSignature(regionsToAvoid())}|${signature(native)}|${stackSize}`;
  };
  /**
   * Whether the stack, where it was placed, now covers something it must keep clear: a native view, the focused
   * element, or a region that is not content scrolled under it. Without a floating placement, it asks for one.
   */
  const coversSomething = () => {
    const at = placed.current;
    const stack = ref.current;
    if (at === null || stack === null) return true;
    const size = stack.getBoundingClientRect();
    const viewport = { width: window.innerWidth, height: window.innerHeight, leftInset: at.inset };
    const rect = stackRect(
      at.corner,
      viewport,
      { width: size.width, height: size.height },
      at.lift,
    );
    const focused = document.activeElement;
    const keep = regionsToAvoid()
      .filter(({ element }) => element === focused || !inScrolled(element, scrolled.current))
      .map((region) => region.rect);
    return coversAny(rect, [...keep, ...nativeToAvoid()]);
  };

  useEffect(() => {
    const bump = () => {
      setTick((n) => n + 1);
    };
    // Focus moves the stack only when it now covers something to keep clear (the focused field counts even in
    // scrolled content); elsewhere the stack stays where it is.
    const check = () => {
      if (!dockedNow.current && coversSomething()) bump();
    };
    // Not on scroll: content scrolling under a toast is the user moving it, and re-placing on every scroll made
    // the stack jump between corners. A scroll only marks its container, so what it holds no longer counts.
    // Only containers the user can scroll are marked: a clipped frame that a focus change scrolls into view
    // (the conversation view holding the message box) keeps its content protected.
    const note = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Element)) {
        scrolled.current.add(document.documentElement);
        return;
      }
      const style = window.getComputedStyle(target);
      if (/auto|scroll|overlay/.test(`${style.overflowX} ${style.overflowY}`))
        scrolled.current.add(target);
    };
    const events = ["resize", NATIVE_LAYOUT_EVENT];
    const focus = ["focusin", "focusout"];
    for (const type of events) window.addEventListener(type, bump, true);
    for (const type of focus) window.addEventListener(type, check, true);
    window.addEventListener("scroll", note, true);
    return () => {
      for (const type of events) window.removeEventListener(type, bump, true);
      for (const type of focus) window.removeEventListener(type, check, true);
      window.removeEventListener("scroll", note, true);
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
    scrolled.current = new WeakSet();
    placed.current = null;
    if (dockedNow.current) {
      reserve(RESERVATION, null);
      return;
    }
    const regions = regionsToAvoid();
    const inset = Math.max(
      18,
      ...regions
        .filter(({ element }) =>
          element.matches('[data-toast-avoid="rail"], [data-toast-avoid="strip"]'),
        )
        .map(({ rect }) => rect.right + 8),
    );
    const size = stack.getBoundingClientRect();
    const viewport = { width: window.innerWidth, height: window.innerHeight, leftInset: inset };
    const stackSize = { width: size.width, height: size.height };
    const native = nativeToAvoid();
    const avoid = regions.map((region) => region.rect);
    const placement = placeToastsOver(viewport, stackSize, avoid, native);
    if (placement.blocked) {
      dockedNow.current = true;
      setDocked(true);
      reserve(RESERVATION, null);
      return;
    }
    placed.current = { corner: placement.corner, lift: placement.lift, inset };
    setCorner(placement.corner);
    setLift(placement.lift);
    setLeftInset(inset);
    reserve(RESERVATION, null);
  }, [list, tick]);

  // while toasts show, a region to keep clear that appears or changes size (the layout switching after a resize,
  // the composer growing) is checked against the stack, which moves only if it now covers it. Only a real
  // change checks: the observer's first report after `observe` finds the same regions, so this cannot loop.
  const showing = list.length > 0;
  useEffect(() => {
    if (!showing) return;
    const bumpIfChanged = () => {
      const next = measured();
      if (next !== placedFor.current) {
        placedFor.current = next;
        if (!dockedNow.current && coversSomething()) setTick((n) => n + 1);
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
              tooltip={false}
              data-testid="shell.toasts-set-selected-id"
              icon="arrow-left"
              label="Previous notification"
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
              tooltip={false}
              data-testid="shell.toasts-set-selected-id-2"
              icon="chevron-right"
              label="Next notification"
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
