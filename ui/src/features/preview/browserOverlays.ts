/**
 * Browser chrome overlay registry shared by menus, selectors and future suggestions/permission popovers.
 * Depends on: DOM, native layout notifications. Called by: useBrowserPopup, BrowserOverlayLayer.
 * Invariant: overlays never reserve or subtract page geometry; nested overlays dismiss from the inside out.
 */
import { NATIVE_LAYOUT_EVENT } from "../../components/nativeLayer";

interface Entry {
  element: HTMLElement;
  trigger: HTMLElement | null;
  close: () => void;
}
const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();
let revision = 0;
let handoff: Promise<void> = Promise.resolve();

function changed(): void {
  revision++;
  for (const listener of listeners) listener();
}

function outside(event: MouseEvent): void {
  const target = event.target;
  if (!(target instanceof Node)) return;
  const open = Array.from(entries.values());
  let within = -1;
  open.forEach(({ element, trigger }, index) => {
    if (element.contains(target) || trigger?.contains(target)) within = index;
  });
  const dismissed = open.slice(within + 1);
  const opener = dismissed[0]?.trigger;
  for (const entry of dismissed.reverse()) entry.close();
  opener?.focus();
}

function escape(event: KeyboardEvent): void {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  if (event.target instanceof Element && event.target.closest('[aria-modal="true"]')) return;
  const last = Array.from(entries.values()).at(-1);
  if (last === undefined) return;
  event.preventDefault();
  last.close();
  last.trigger?.focus();
}

/** Registers a visible chrome overlay, with no effect on the page's layout. Returns its cleanup. */
export function registerBrowserOverlay(key: string, entry: Entry): () => void {
  if (entries.size === 0) {
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape);
  }
  entries.set(key, entry);
  entry.element.dataset.browserOverlayMenu = "true";
  changed();
  return () => {
    if (entries.get(key) !== entry) return;
    entries.delete(key);
    delete entry.element.dataset.browserOverlayMenu;
    changed();
    if (entries.size === 0) {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", escape);
    }
  };
}

/** Focuses the visible innermost menu after the native handoff; editable suggestions retain their input. */
export function focusBrowserOverlay(): void {
  const last = Array.from(entries.values()).at(-1);
  if (last === undefined || last.element.contains(document.activeElement)) return;
  if (last.trigger instanceof HTMLInputElement || last.trigger instanceof HTMLTextAreaElement)
    return;
  last.element.querySelector<HTMLElement>("button:not(:disabled), input, summary")?.focus();
}

/** Stable external-store revision and current open count. */
export const browserOverlayRevision = (): number => revision;
/** Number of live popovers, including nested selector portals. */
export const browserOverlayCount = (): number => entries.size;
/** Observes changes to chrome overlays. */
export function onBrowserOverlays(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
/** Closes all chrome popovers when their underlying page or real layout changes. */
export function closeBrowserOverlays(): void {
  for (const entry of Array.from(entries.values()).reverse()) entry.close();
}
/** Publishes the native handoff so page actions can await complete restoration. */
export function trackBrowserOverlayWork(work: Promise<void>): void {
  handoff = work;
}
/** Dismisses nested chrome before a page action, waiting for React cleanup and native restoration. */
export async function closeBrowserOverlaysAndWait(): Promise<void> {
  closeBrowserOverlays();
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        resolve();
      });
    });
  });
  await handoff;
}
/** Auto-dismiss before a resized page could invalidate its menu placement. */
export function watchBrowserOverlayLayout(): () => void {
  window.addEventListener("resize", closeBrowserOverlays);
  window.addEventListener(NATIVE_LAYOUT_EVENT, closeBrowserOverlays);
  return () => {
    window.removeEventListener("resize", closeBrowserOverlays);
    window.removeEventListener(NATIVE_LAYOUT_EVENT, closeBrowserOverlays);
  };
}
