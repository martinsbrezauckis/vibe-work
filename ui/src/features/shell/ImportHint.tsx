/**
 * The first-start hint about other coding agents' setups: one row on the start screen, shown only when the core's
 * `importSetup detect` (existence checks, no file read) finds another agent's personal setup. "Look at it" opens
 * Settings > General at "Import from another agent" (which reads only when asked there); "Not now"
 * ends the hint. Either answer is remembered on this computer, so the row never comes back.
 * Depends on AppContext (core, shell actions), the shared `key-line` row style and the generated core types.
 * Called by Content (start screen).
 * Invariants: asks `detect` at most once per window run and never after an answer (a mount made while the question
 * runs shares it); shows nothing on a failed check; nothing is brought from here; Not now hands focus to the next
 * control after the row before the row is removed.
 */
import { type ReactElement, useEffect, useState } from "react";

import { Button } from "../../components/Controls";
import type { ImportDetected } from "../../protocol/core";
import { useApp } from "./AppContext";

/** `localStorage` key: "1" after the person answered the hint. */
export const IMPORT_HINT_KEY = "vw.importHintDone";

function answered(): boolean {
  try {
    return localStorage.getItem(IMPORT_HINT_KEY) === "1";
  } catch {
    return false;
  }
}

function remember(): void {
  try {
    localStorage.setItem(IMPORT_HINT_KEY, "1");
  } catch {
    // Not remembered: the hint may show once more at the next start. The app keeps working.
  }
}

/** "Codex", "Codex and Cursor", "Codex, OpenCode and Cursor". */
export function agentList(labels: readonly string[]): string {
  if (labels.length <= 1) return labels.join("");
  return `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1] ?? ""}`;
}

/** The first enabled control after `section` in document order, or null. */
function nextFocusable(section: Element | null): HTMLElement | null {
  if (section === null) return null;
  const controls = document.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
  );
  for (const control of controls)
    if (
      !section.contains(control) &&
      section.compareDocumentPosition(control) & Node.DOCUMENT_POSITION_FOLLOWING
    )
      return control;
  return null;
}

/** This window run's one `detect` question; every mount (also one made while it runs) waits for the same answer. */
let detection: Promise<ImportDetected[] | null> | null = null;

/** Test hook: forget this run's answer from the core. */
export function resetImportHintForTests(): void {
  detection = null;
}

/** The row while another agent's setup was found and the hint was not answered. */
export function ImportHint(): ReactElement | null {
  const { core, shell } = useApp();
  const [agents, setAgents] = useState<ImportDetected[] | null>(null);
  useEffect(() => {
    if (core === null || answered()) return;
    detection ??= core
      .run({ type: "importSetup", request: { op: "detect" } })
      .then((result) =>
        result.type === "importSetup" && result.result.kind === "detected"
          ? result.result.agents
          : null,
      )
      // No hint when the check fails; Settings keeps the same feature.
      .catch(() => null);
    let live = true;
    void detection.then((found) => {
      if (live && !answered()) setAgents(found);
    });
    return () => {
      live = false;
    };
  }, [core]);
  if (agents === null || agents.length === 0) return null;
  const end = (look: boolean, from: HTMLElement) => {
    // The row goes away with focus inside it: Not now hands focus to the next control after the row before the row
    // is removed, so focus is never left on nothing (a screen reader would land on the page's main region). Look
    // at it opens Settings, which focuses the found setting.
    if (!look) nextFocusable(from.closest("section"))?.focus();
    remember();
    setAgents(null);
    if (look) shell.actions.openSettings("general", "import-setup");
  };
  return (
    <section
      className="key-line import-hint"
      aria-label="Setup from another agent"
      data-testid="shell.import-hint"
    >
      <p>
        Found a setup from {agentList(agents.map((a) => a.label))} on this computer. Bring its
        instructions, skills and connectors to Vibe Work?
      </p>
      <Button
        data-testid="shell.import-hint-look"
        type="button"
        shape="outlined"
        onClick={(e) => {
          end(true, e.currentTarget);
        }}
      >
        Look at it
      </Button>
      <Button
        data-testid="shell.import-hint-not-now"
        type="button"
        shape="outlined"
        onClick={(e) => {
          end(false, e.currentTarget);
        }}
      >
        Not now
      </Button>
    </section>
  );
}
