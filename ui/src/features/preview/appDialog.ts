/** Reports the visible app modal's accessible name through the typed core client.
 * Used by preview registration; native modal dialogs have independent lifetime guards in Rust.
 */
import type { CoreController } from "../../state/coreStore";

/** The topmost visible dialog name, excluding inert dialogs underneath it. */
export function openAppDialog(): string | null {
  const dialogs = document.querySelectorAll<HTMLElement>(
    '[role="dialog"][aria-modal="true"], [role="alertdialog"], dialog[open]',
  );
  for (const dialog of Array.from(dialogs).reverse()) {
    if (dialog.closest('[hidden], [inert], [aria-hidden="true"]')) continue;
    let hidden = false;
    for (let node: HTMLElement | null = dialog; node; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (style.display === "none" || style.visibility === "hidden") hidden = true;
    }
    if (hidden) continue;
    const ids = dialog.getAttribute("aria-labelledby")?.split(/\s+/) ?? [];
    const labelled = ids.map((id) => document.getElementById(id)?.textContent ?? "").join(" ");
    const name = (labelled.trim() || dialog.getAttribute("aria-label") || "Vibe Work")
      .replace(/\s+/g, " ")
      .trim();
    return Array.from(name).slice(0, 120).join("");
  }
  return null;
}

/** Serial reporting prevents a delayed open notification from restoring a closed dialog. */
export function reportAppDialog(core: Pick<CoreController, "run">): () => void {
  let latest = openAppDialog();
  let sent: string | null | undefined;
  let sending = false;
  let active = true;
  const flush = async () => {
    if (sending) return;
    sending = true;
    try {
      while (active && latest !== sent) {
        const name = latest;
        await core.run({ type: "preview", request: { op: "appDialog", name } });
        sent = name;
      }
    } catch {
      // The next modal/window change retries; native input keeps its independent visibility guards.
    } finally {
      sending = false;
    }
  };
  const changed = () => {
    latest = openAppDialog();
    void flush();
  };
  const observer = new MutationObserver(changed);
  observer.observe(document.body, {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
    attributeFilter: [
      "role",
      "aria-modal",
      "aria-label",
      "aria-labelledby",
      "aria-hidden",
      "hidden",
      "inert",
      "open",
      "style",
      "class",
    ],
  });
  changed();
  return () => {
    active = false;
    observer.disconnect();
  };
}
