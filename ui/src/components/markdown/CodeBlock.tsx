/**
 * A fenced code block of model output: language label, copy button and syntax highlighting. The code
 * is always text: plain first, then highlighted tokens as `<span class>` elements once the language
 * is loaded (`highlight.ts`). HTML inside the code stays text.
 * Depends on: highlight.ts, CopyButton. Called by: Markdown.
 * Invariants: no `style` attribute and no HTML string; while the message is still streaming the block
 * is not highlighted (it changes on every patch), so a streaming row stays cheap.
 */
import { type ReactElement, useEffect, useState } from "react";

import { CopyButton } from "../CopyButton";
import { type CodeToken, cachedTokens, highlight, languageOf } from "./highlight";

/** Props of [`CodeBlock`]. */
export interface CodeBlockProps {
  code: string;
  /** The fence's language name, if any (`ts`, `python`, ...). */
  fence: string | null;
  /** The message is still being generated: skip highlighting for now. */
  streaming?: boolean;
}

/** Renders one code block. */
export function CodeBlock({ code, fence, streaming = false }: CodeBlockProps): ReactElement {
  const lang = languageOf(fence);
  const key = `${lang ?? ""}\u0000${code}`;
  // Highlighted lines arrive asynchronously; they count only for the code they were made for.
  const [result, setResult] = useState<{ key: string; lines: CodeToken[][] } | null>(null);
  useEffect(() => {
    if (lang === null || streaming) return;
    let live = true;
    void highlight(code, lang).then((lines) => {
      if (live && lines !== null) setResult({ key, lines });
    });
    return () => {
      live = false;
    };
  }, [code, lang, streaming, key]);
  const tokens =
    lang === null || streaming
      ? null
      : result?.key === key
        ? result.lines
        : cachedTokens(code, lang);

  return (
    <div className="md-code">
      <div className="md-code-bar">
        <span className="md-code-lang">{fence ?? "text"}</span>
        <CopyButton testId="shared.code-block-copybutton" text={code} label="Copy code" />
      </div>
      <pre>
        <code data-lang={lang ?? undefined}>
          {tokens === null
            ? code
            : tokens.map((line, i) => (
                <span key={i} className="md-code-line">
                  {line.map((token, j) =>
                    token.className === "" ? (
                      token.text
                    ) : (
                      <span key={j} className={token.className}>
                        {token.text}
                      </span>
                    ),
                  )}
                  {i < tokens.length - 1 ? "\n" : null}
                </span>
              ))}
        </code>
      </pre>
    </div>
  );
}
