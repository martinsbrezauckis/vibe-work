/**
 * Hides background scrollbars while a modal is open, including scroll containers mounted later.
 * Depends on the DOM; called by useModalFocus. Nested dialogs keep the background locked until the last closes.
 */
const dialogs: HTMLElement[] = [];
const locked = new Map<
  HTMLElement,
  { overflow: { name: string; value: string; priority: string }[]; top: number; left: number }
>();
let observer: MutationObserver | null = null;

function restore(element: HTMLElement): void {
  const saved = locked.get(element);
  if (!saved) return;
  element.style.removeProperty("overflow");
  for (const { name, value, priority } of saved.overflow) {
    if (value) element.style.setProperty(name, value, priority);
  }
  element.scrollTop = saved.top;
  element.scrollLeft = saved.left;
  locked.delete(element);
}

function insideModal(element: HTMLElement, dialog: HTMLElement): boolean {
  if (dialog.contains(element)) return true;
  const trigger = element.closest<HTMLElement>("[data-selector-trigger]")?.dataset.selectorTrigger;
  const owner = trigger ? document.getElementById(trigger) : null;
  return owner !== null && dialog.contains(owner);
}

function sync(): void {
  const dialog = dialogs.at(-1);
  for (const element of locked.keys()) {
    if (!dialog || insideModal(element, dialog)) restore(element);
  }
  if (!dialog) return;
  for (const element of document.querySelectorAll<HTMLElement>("*")) {
    if (insideModal(element, dialog) || locked.has(element)) continue;
    const style = getComputedStyle(element);
    if (
      ![style.overflow, style.overflowX, style.overflowY].some((v) => /auto|scroll|overlay/.test(v))
    )
      continue;
    locked.set(element, {
      overflow: ["overflow", "overflow-x", "overflow-y"].map((name) => ({
        name,
        value: element.style.getPropertyValue(name),
        priority: element.style.getPropertyPriority(name),
      })),
      top: element.scrollTop,
      left: element.scrollLeft,
    });
    element.style.setProperty("overflow", "hidden", "important");
  }
}

/** Locks scrollers outside this modal and returns its release function. */
export function lockModalScroll(dialog: HTMLElement): () => void {
  dialogs.push(dialog);
  sync();
  if (!observer) {
    observer = new MutationObserver(sync);
    observer.observe(document.body, { childList: true, subtree: true });
  }
  return () => {
    dialogs.splice(dialogs.indexOf(dialog), 1);
    if (dialogs.length === 0) {
      observer?.disconnect();
      observer = null;
    }
    sync();
  };
}
