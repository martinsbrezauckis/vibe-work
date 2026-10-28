/** Shared visible-element check for Debug API DOM routes. Uses measured boxes and ancestor styles;
 * controls, clicks and rectangle reads must agree when Settings hides the underlying page. */
export function elementVisible(element: Element): boolean {
  const box = element.getBoundingClientRect();
  if (box.width < 1 || box.height < 1) return false;
  // Hover-revealed controls still occupy a clickable box before the pointer arrives.
  for (let up: Element | null = element; up; up = up.parentElement) {
    const style = getComputedStyle(up);
    if (
      up.hasAttribute("hidden") ||
      up.hasAttribute("inert") ||
      style.display === "none" ||
      style.visibility === "hidden" ||
      style.visibility === "collapse"
    )
      return false;
  }
  return true;
}
