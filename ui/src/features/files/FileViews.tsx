/**
 * The File tab's body per kind: text and code (highlighted with the app's CSP-safe tokenizer up to
 * `HIGHLIGHT_LIMIT` characters, plain text above, line numbers, scroll to a line), Markdown (the app's safe
 * Markdown module: no raw HTML, images not loaded, links through the link dialog), images (`data:` URL the core
 * read after the scope check), and the notice for files shown in the isolated viewer or not shown at all.
 * Depends on: components/markdown (Markdown, highlight), FileRef (references inside a Markdown file), model/kinds.
 * Called by: FileTab.
 * Invariants: every piece of file content is text in React elements (never HTML, never a `style` attribute); HTML
 * and PDF files are never rendered in this window (the viewer webview shows them).
 */
import { type ReactElement, useEffect, useRef, useState } from "react";

import { Markdown } from "../../components/markdown/Markdown";
import { type CodeToken, highlight, languageOf } from "../../components/markdown/highlight";
import type { FileContent } from "../../protocol/core";
import { FILE_REFS } from "./FileRef";
import { languageForName } from "./model/kinds";

/** Largest text the File tab highlights (never the whole of a big file). */
export const HIGHLIGHT_LIMIT = 100_000;

/** Highlighted lines of `text` once ready (`null`: plain). */
function useTokens(text: string, lang: string | null): CodeToken[][] | null {
  const [result, setResult] = useState<{ text: string; lines: CodeToken[][] } | null>(null);
  useEffect(() => {
    if (lang === null) return;
    let live = true;
    void highlight(text, lang, HIGHLIGHT_LIMIT).then((lines) => {
      if (live && lines !== null) setResult({ text, lines });
    });
    return () => {
      live = false;
    };
  }, [text, lang]);
  return result?.text === text ? result.lines : null;
}

/** Text with line numbers; scrolls `line` into view once shown. */
export function TextView({
  content,
  line,
  end,
}: {
  content: FileContent;
  line: number | null;
  end?: number;
}): ReactElement {
  const text = content.text ?? "";
  const lang = languageOf(languageForName(content.name));
  const tokens = useTokens(text, lang);
  const all = tokens ?? text.split("\n").map((l) => [{ text: l, className: "" }]);
  // A file ending with a newline has no line after it (no empty numbered line at the end).
  const lines = text.endsWith("\n") && all.length > 1 ? all.slice(0, -1) : all;
  const shorter = end !== undefined && Math.max(line ?? 0, end) > lines.length;
  const first = shorter ? 1 : line;
  const last = shorter ? 1 : (end ?? line);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (first === null) return;
    box.current
      ?.querySelector(`[data-line="${String(first)}"]`)
      ?.scrollIntoView({ block: "center" });
  }, [first, content.fingerprint]);
  return (
    <div className="file-text" ref={box} data-lang={lang ?? "text"}>
      {shorter && (
        <p className="file-note" role="status">
          This file is shorter now than when the lines were pointed at.
        </p>
      )}
      <pre>
        <code>
          {lines.map((parts, i) => (
            <span
              key={i}
              className={`file-line${first !== null && last !== null && i + 1 >= first && i + 1 <= last ? " at" : ""}`}
              data-line={i + 1}
            >
              <span className="file-ln" aria-hidden="true">
                {i + 1}
              </span>
              {parts.map((p, j) =>
                p.className === "" ? (
                  p.text
                ) : (
                  <span key={j} className={p.className}>
                    {p.text}
                  </span>
                ),
              )}
              {"\n"}
            </span>
          ))}
        </code>
      </pre>
    </div>
  );
}

/** A Markdown file, rendered like the transcript renders the agent's text, but with the standard line rules (a
 * single newline inside a paragraph is a space, not a line break as in chat). */
export function MarkdownView({
  content,
  onOpenLink,
}: {
  content: FileContent;
  onOpenLink: (url: string) => void;
}): ReactElement {
  return (
    <div className="file-md">
      <Markdown source={content.text ?? ""} onOpenLink={onOpenLink} extension={FILE_REFS} />
    </div>
  );
}

/** An image the core read (PNG, JPEG, GIF, WebP, SVG as a `data:` URL; an SVG in `<img>` runs nothing). */
export function ImageView({ content }: { content: FileContent }): ReactElement {
  if (content.tooLarge || content.dataUrl === null)
    return (
      <p className="file-note">
        This picture is too large to show here (over 20 MB). Use Open with default app.
      </p>
    );
  return (
    <div className="file-image">
      <img src={content.dataUrl} alt={content.name} />
    </div>
  );
}

/** A file not shown in the window. */
export function BinaryView({ content }: { content: FileContent }): ReactElement {
  return (
    <p className="file-note">
      This file is not text Vibe Work can show ({content.name}). Use Show in folder, or Open with
      default app when the button is offered.
    </p>
  );
}
