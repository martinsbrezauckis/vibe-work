/**
 * Reading policy shared by the toast view and its tests. Depends on notification types; called by
 * ToastItem. Information and warnings kept in the bell leave after a reading time (the bell keeps them); errors,
 * and warnings the bell does not keep, stay until dismissed.
 */
import { isBellKind, type ToastSpec } from "./notifications";

/**
 * Milliseconds to read a toast at roughly 200 words/minute, with time to notice it, or `null` when it stays until
 * dismissed. A warning gets a longer minimum and leaves only when the bell keeps it, so it is never lost.
 */
export function toastLifetime({ kind, text, bellKind }: ToastSpec): number | null {
  if (kind === "error" || (kind === "warn" && !isBellKind(bellKind))) return null;
  const words = text.trim().split(/\s+/u).filter(Boolean).length;
  // Long paths and identifiers also take reading time even without whitespace.
  const units = Math.max(words, Math.ceil(Array.from(text).length / 6));
  const read = 2000 + units * 300;
  return kind === "warn" ? Math.max(8000, read + 3000) : Math.max(5000, read);
}

/** Conservative prefix before layout is available; complete text always remains in history. */
export function initialToastPreview(text: string): string {
  const lines = text.split("\n").slice(0, 3).join("\n");
  return Array.from(lines).slice(0, 160).join("");
}

/**
 * Fits an exact prefix into three rendered lines using a hidden measuring span, without CSS
 * clipping or ellipsis. Width/font changes are handled by the calling view's ResizeObserver.
 */
export function measureToastPreview(text: string, element: HTMLElement): string {
  const width = element.getBoundingClientRect().width;
  if (width <= 0) return initialToastPreview(text);
  const style = window.getComputedStyle(element);
  const lineHeight = Number.parseFloat(style.lineHeight);
  if (!Number.isFinite(lineHeight) || lineHeight <= 0) return initialToastPreview(text);
  const probe = document.createElement("span");
  probe.className = "toast-text toast-measure";
  probe.style.width = `${String(width)}px`;
  probe.style.font = style.font;
  probe.style.lineHeight = style.lineHeight;
  element.parentElement?.append(probe);
  const chars = Array.from(text);
  const fits = (length: number) => {
    probe.textContent = chars.slice(0, length).join("");
    return probe.getBoundingClientRect().height <= lineHeight * 3 + 0.5;
  };
  let low = 0;
  let high = chars.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (fits(middle)) low = middle;
    else high = middle - 1;
  }
  probe.remove();
  return chars.slice(0, low).join("");
}
