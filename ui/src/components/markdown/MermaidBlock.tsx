/** Lazy Mermaid fence presentation. Uses passive XML conversion and source-based CopyButton; failures stay code. */
import { type ReactElement, type ReactNode, useEffect, useId, useRef, useState } from "react";
import { CodeBlock } from "./CodeBlock";
import { drawingFloorClasses } from "./diagramFloor";
import { CopyButton } from "../CopyButton";

const REFUSALS = new Set(["DiagramInputError", "DiagramLimitError", "DiagramUnsupportedError"]);

/** The drawing code's own refusal sentence (it names the line or the bound); anything else gets no guessed reason. */
function refusalReason(error: unknown): string | null {
  if (!(error instanceof Error) || !REFUSALS.has(error.name)) return null;
  const text = error.message.trim().replace(/[.\s]+$/, "");
  return text === "" ? null : `${text.length > 140 ? `${text.slice(0, 140)}…` : text}.`;
}

const scrollLabel = {
  x: "Diagram, scroll sideways to see all",
  y: "Diagram, scroll up and down to see all",
  both: "Diagram, scroll to see all",
} as const;

/** Draw a finished fence; incomplete streaming source stays cheap and copyable. */
export function MermaidBlock({
  source,
  streaming,
}: {
  source: string;
  streaming: boolean;
}): ReactElement {
  const id = useId().replace(/[^a-z0-9]/gi, "");
  const [result, setResult] = useState<{
    source: string;
    drawing: ReactNode;
    floor: string | null;
    reason: string | null;
  } | null>(null);
  const [scrolls, setScrolls] = useState<"x" | "y" | "both" | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (streaming) return;
    let live = true;
    void Promise.all([import("./diagramRender"), import("./passiveMarkup")])
      .then(async ([renderer, markup]) => {
        const svg = await renderer.drawDiagram(source);
        return {
          drawing: markup.passiveMarkup(svg, "svg", id, source),
          floor: drawingFloorClasses(svg),
        };
      })
      .then(({ drawing, floor }) => {
        if (live) setResult({ source, drawing, floor, reason: null });
      })
      .catch((error: unknown) => {
        if (live) setResult({ source, drawing: null, floor: null, reason: refusalReason(error) });
      });
    return () => {
      live = false;
    };
  }, [source, streaming, id]);
  const current = !streaming && result?.source === source ? result : null;
  const drawing = current?.drawing ?? null;
  const drawn = drawing !== null;
  // The area is a keyboard stop only while something is hidden beyond its edge.
  useEffect(() => {
    const area = scroller.current;
    if (!drawn || !area) return;
    const measure = (): void => {
      const across = area.scrollWidth > area.clientWidth + 1;
      const down = area.scrollHeight > area.clientHeight + 1;
      setScrolls(across && down ? "both" : across ? "x" : down ? "y" : null);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(area);
    return () => {
      observer.disconnect();
    };
  }, [drawn, drawing, current?.floor]);
  if (!current?.drawing)
    return (
      <>
        {/* The reason comes first: a refused diagram can have a very long source below it. */}
        {current !== null ? (
          <p className="md-render-error">
            <span>Could not draw this diagram.</span> {current.reason}
            {current.reason === null ? "" : " "}
            Ask the agent to correct it, or copy the source below.
          </p>
        ) : null}
        <CodeBlock code={source} fence="mermaid" streaming={streaming} />
      </>
    );
  return (
    <div className={`md-code md-diagram ${current.floor ?? ""}`.trim()}>
      <div className="md-code-bar">
        <span>mermaid</span>
        <CopyButton testId="shared.diagram-copy" text={source} label="Copy diagram source" />
      </div>
      <div
        ref={scroller}
        className="md-diagram-drawing"
        {...(scrolls ? { role: "region", tabIndex: 0, "aria-label": scrollLabel[scrolls] } : {})}
      >
        <div role="img" aria-label={source}>
          {current.drawing}
        </div>
      </div>
    </div>
  );
}
