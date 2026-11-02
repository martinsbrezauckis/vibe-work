/** Keyboard traversal of visible app regions. Called by shell wiring; modal focus remains inside its dialog. */
export function startRegionNavigation(): () => void {
  const onKey = (event: KeyboardEvent) => {
    if (
      event.key !== "F6" ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.defaultPrevented
    )
      return;
    if (document.querySelector('[aria-modal="true"], dialog[open]')) return;
    const regions = [...document.querySelectorAll<HTMLElement>("[data-focus-region]")].filter(
      (node) => {
        if (node.closest('[inert], [hidden], [aria-hidden="true"]')) return false;
        return node.getClientRects().length > 0 && getComputedStyle(node).visibility !== "hidden";
      },
    );
    if (regions.length === 0) return;
    const active = document.activeElement;
    // Nested regions (transcript and composer inside main) take precedence over main.
    let current = -1;
    regions.forEach((node, index) => {
      if (node === active || (active !== null && node.contains(active))) current = index;
    });
    const index =
      current < 0
        ? event.shiftKey
          ? regions.length - 1
          : 0
        : (current + (event.shiftKey ? -1 : 1) + regions.length) % regions.length;
    event.preventDefault();
    regions[index]?.focus();
  };
  window.addEventListener("keydown", onKey);
  return () => {
    window.removeEventListener("keydown", onKey);
  };
}
