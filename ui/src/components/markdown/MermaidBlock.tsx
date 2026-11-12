/** Lazy Mermaid fence presentation. Uses passive XML conversion and source-based CopyButton; failures stay code. */
import { type ReactElement, type ReactNode, useEffect, useId, useState } from "react";
import { CodeBlock } from "./CodeBlock";
import { CopyButton } from "../CopyButton";

/** Draw a finished fence; incomplete streaming source stays cheap and copyable. */
export function MermaidBlock({
  source,
  streaming,
}: {
  source: string;
  streaming: boolean;
}): ReactElement {
  const id = useId().replace(/[^a-z0-9]/gi, "");
  const [result, setResult] = useState<{ source: string; drawing: ReactNode } | null>(null);
  useEffect(() => {
    if (streaming) return;
    let live = true;
    void Promise.all([import("./diagramRender"), import("./passiveMarkup")])
      .then(async ([renderer, markup]) =>
        markup.passiveMarkup(await renderer.drawDiagram(source), "svg", id, source),
      )
      .then((drawing) => {
        if (live) setResult({ source, drawing });
      })
      .catch(() => {
        if (live) setResult({ source, drawing: null });
      });
    return () => {
      live = false;
    };
  }, [source, streaming, id]);
  const current = !streaming && result?.source === source ? result : null;
  if (!current?.drawing)
    return (
      <>
        <CodeBlock code={source} fence="mermaid" streaming={streaming} />
        {current !== null ? <p className="md-render-error">Could not draw this diagram.</p> : null}
      </>
    );
  return (
    <div className="md-code md-diagram">
      <div className="md-code-bar">
        <span>mermaid</span>
        <CopyButton testId="shared.diagram-copy" text={source} label="Copy diagram source" />
      </div>
      <div className="md-diagram-drawing" role="img" aria-label={source}>
        {current.drawing}
      </div>
    </div>
  );
}
