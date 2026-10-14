/**
 * A small "Copy" button adapted from ShellX's CopyableCodeBlock. It copies the source string
 * instead of rendered `innerText`, which depends on layout, and reports clipboard failures.
 * Depends on: react, the Clipboard API. Called by: code blocks, message rows, the link dialog.
 * Invariants: the label says what happened for 1.2 s ("Copied" / "Copy failed"), then returns.
 */
import { type ReactElement, useEffect, useRef, useState } from "react";

/** Copies `text` when clicked. `label` names what is copied (for the accessible name). */
export function CopyButton({
  text,
  label = "Copy",
}: {
  text: string;
  label?: string;
}): ReactElement {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current);
    },
    [],
  );
  const show = (next: "copied" | "failed") => {
    setState(next);
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      setState("idle");
    }, 1200);
  };
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      show("copied");
    } catch {
      show("failed");
    }
  };
  const actionLabel =
    state === "idle"
      ? label
      : state === "copied"
        ? `${label} (copied)`
        : `${label} (copy failed; try again)`;
  return (
    <button
      type="button"
      className={`copy-btn${state === "idle" ? "" : ` ${state}`}`}
      aria-label={actionLabel}
      title={actionLabel}
      onClick={() => void copy()}
    >
      {/* the copy picture turns into a check (150 ms) while "Copied" shows. Drawn in CSS, not SVG: this
          button sits inside rendered Markdown, where the trusted-surface tests allow no SVG at all. */}
      <span className="copy-ic" aria-hidden="true" />
      {state === "idle" ? "Copy" : state === "copied" ? "Copied" : "Copy failed"}
    </button>
  );
}
