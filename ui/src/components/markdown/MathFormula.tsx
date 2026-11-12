/** Accessible formula presentation. Lazy KaTeX output becomes allowlisted native MathML; source remains the fallback. */
import { type ReactElement, type ReactNode, useEffect, useState } from "react";

/** Inline/display math with a source label; stale async results never replace a newer expression. */
export function MathFormula({
  source,
  raw,
  display,
}: {
  source: string;
  raw: string;
  display: boolean;
}): ReactElement {
  const key = `${String(display)}:${source}`;
  const [result, setResult] = useState<{ key: string; math: ReactNode } | null>(null);
  useEffect(() => {
    let live = true;
    void Promise.all([import("./mathRender"), import("./passiveMarkup")])
      .then(([renderer, markup]) =>
        markup.passiveMarkup(renderer.drawMath(source, display), "math", "", source),
      )
      .then((math) => {
        if (live) setResult({ key, math });
      })
      .catch(() => {
        if (live) setResult({ key, math: null });
      });
    return () => {
      live = false;
    };
  }, [key, source, display]);
  return (
    <span className="md-math" data-display={display ? "display" : "inline"}>
      {result?.key === key && result.math !== null ? result.math : raw}
    </span>
  );
}
