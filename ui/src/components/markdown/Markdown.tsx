/**
 * The one Markdown renderer for untrusted text (model output, tool output, file previews).
 * Trusted-surface rule (decided 2026-10-08): the main window never renders untrusted
 * active content, so this renderer has NO path to raw HTML.
 *
 * - `react-markdown` 10.1.0 builds React elements (it never sets inner HTML); `remark-gfm`
 *   adds tables, task lists and strikethrough. No `rehype-raw`, no `rehype-*` HTML plugin.
 * - `skipHtml`: raw HTML in the text is dropped, not shown and not escaped into the page.
 * - The default `urlTransform` stays: it blocks `javascript:`, `data:` and other non-web schemes.
 * - Images are not loaded at all (an image is a request to a remote host: data leaving the
 *   machine, an action requiring confirmation); the alt text is shown instead. An extension may show a local
 *   session file the core resolved and read (`MarkdownExtension.image`); never a URL.
 * - Links never navigate the main window: the click is cancelled and `onOpenLink` decides what
 *   happens (the transcript shows the URL in full first). Nothing here can reach the core.
 * - Fenced code goes to `CodeBlock` (copy button, highlighting as classes).
 * - `streaming`: while Vibe is still generating the text, `remend` 1.4.0 closes what a
 *   half-written message leaves open (`**`, code fences, links: link text only, never a placeholder
 *   URL) so the partial text renders as it will look, without flicker into raw syntax. `remend` is
 *   its own chunk (main bundle size): it starts loading with this module and a streaming
 *   message renders its plain text until it arrived (a few milliseconds after start).
 *
 * - `lineBreaks`: chat text (the transcript's messages): a single newline is a line break, as in chat apps, instead
 *   of the standard soft break shown as a space (`remarkLineBreaks.ts`). Off for Markdown files, which keep the
 *   standard rules.
 * - `extension`: one extra remark plugin and the element it produces, so the transcript can mark file
 *   references (`features/files/remarkFileRefs.ts`) without this module knowing about files. The plugin may only
 *   add elements of its own tag; it never adds HTML.
 *
 * Depends on: react-markdown, remark-gfm, remend, CodeBlock, remarkLineBreaks.
 * Called by: the transcript rows, previews.
 */
import { type ReactElement, type ReactNode, useEffect, useId, useState } from "react";
import ReactMarkdown, { type Options } from "react-markdown";
import remarkGfm from "remark-gfm";

import { CodeBlock } from "./CodeBlock";
import { remarkLineBreaks } from "./remarkLineBreaks";

/** Props of [`Markdown`]. */
export interface MarkdownProps {
  /** The Markdown source (untrusted). */
  source: string;
  /** Called with a link's URL when the user clicks it; the window itself never navigates. */
  onOpenLink?: (url: string) => void;
  /** The text is still being generated (Vibe's `generationStatus: in_progress`). */
  streaming?: boolean;
  /** Chat text: a single newline shows as a line break (see the module notes). */
  lineBreaks?: boolean;
  /** An extra remark plugin and the renderer of the element it adds (for file references). */
  extension?: MarkdownExtension;
}

/** One extra remark plugin and its element (see the module notes). */
export interface MarkdownExtension {
  /** The remark plugin. */
  plugin: NonNullable<Options["remarkPlugins"]>[number];
  /** The element name the plugin produces (`data.hName`). */
  tag: string;
  /** Renders one element from its hast properties and its rendered children. */
  render: (properties: Record<string, unknown>, children: ReactNode) => ReactElement;
  /**
   * shows a Markdown image whose `src` is a LOCAL path (a session file), or returns `undefined` to keep the
   * alt text. The extension resolves the path through the core; this module never loads a URL itself.
   */
  image?: (src: string, alt: string) => ReactElement | undefined;
}

/** GFM task inputs stay read-only; each carries its own item identity independently of visible text. */
function TaskCheckbox({ checked }: { checked?: boolean | undefined }): ReactElement {
  const id = useId();
  return (
    <input
      data-testid="shared.markdown-task"
      data-item-id={id}
      type="checkbox"
      checked={checked}
      disabled
      readOnly
    />
  );
}

const REL = "noopener noreferrer nofollow";

/** A hast node as react-markdown hands it to component overrides (only what is read here). */
interface HastNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
}

function textOf(node: HastNode | undefined): string {
  if (node === undefined) return "";
  if (node.type === "text") return node.value ?? "";
  return (node.children ?? []).map(textOf).join("");
}

/** The fence language from a `<code class="language-xyz">` child of `<pre>`. */
function fenceOf(code: HastNode | undefined): string | null {
  const names = code?.properties?.["className"];
  const list = Array.isArray(names) ? names : [];
  const lang = list.find((n): n is string => typeof n === "string" && n.startsWith("language-"));
  return lang === undefined ? null : lang.slice("language-".length);
}

/** `remend`, once its chunk has loaded. */
let remend: typeof import("remend").default | null = null;
const remendLoaded = import("remend").then((module) => {
  remend = module.default;
});

/** `remend` for a streaming message; re-renders the caller once it has loaded. */
function useRemend(streaming: boolean): typeof remend {
  const [, setLoaded] = useState(remend !== null);
  useEffect(() => {
    if (!streaming || remend !== null) return;
    let live = true;
    void remendLoaded.then(() => {
      if (live) setLoaded(true);
    });
    return () => {
      live = false;
    };
  }, [streaming]);
  return remend;
}

/** Renders `source` as safe Markdown. */
export function Markdown({
  source,
  onOpenLink,
  streaming = false,
  lineBreaks = false,
  extension,
}: MarkdownProps): ReactElement {
  const close = useRemend(streaming);
  const text =
    streaming && close !== null
      ? close(source, { linkMode: "text-only", katex: false, inlineKatex: false, images: false })
      : source;
  // Line breaks run before the extension, so file references see one line per text node.
  const plugins: NonNullable<Options["remarkPlugins"]> = [remarkGfm];
  if (lineBreaks) plugins.push(remarkLineBreaks as (typeof plugins)[number]);
  if (extension !== undefined) plugins.push(extension.plugin);
  return (
    <ReactMarkdown
      remarkPlugins={plugins}
      skipHtml
      components={{
        input: ({ checked }) => <TaskCheckbox checked={checked} />,
        // Only present with an extension; react-markdown looks custom tags up by name.
        ...(extension === undefined
          ? {}
          : {
              [extension.tag]: ({ node, children }: { node?: HastNode; children?: ReactNode }) =>
                extension.render(node?.properties ?? {}, children),
            }),
        a: ({ href, children }) => (
          <a
            data-testid="shared.markdown-prevent-default"
            data-item-id={href ?? "inert"}
            // The default urlTransform turns an unsafe URL into ""; an empty href would link to
            // the current page, so it is dropped (the anchor stays inert).
            {...(href === undefined || href === "" ? {} : { href })}
            rel={REL}
            {...(href === undefined || href === "" || onOpenLink === undefined
              ? {}
              : {
                  title: `Review link before opening: ${href}`,
                  "aria-label": `Review link before opening: ${href}`,
                })}
            onClick={(event) => {
              // Cancel navigation: the main window stays on the app's own origin.
              event.preventDefault();
              if (href !== undefined && href !== "") onOpenLink?.(href);
            }}
          >
            {children}
          </a>
        ),
        // Remote images never load (see the header). An extension may show a LOCAL image it resolved through the
        // core (a session file the agent saved, `![boat](images/boat.png)`); anything else stays text.
        img: ({ src, alt }) =>
          (typeof src === "string" ? extension?.image?.(src, alt ?? "") : undefined) ?? (
            <span className="md-image-blocked">{alt ?? ""}</span>
          ),
        pre: ({ node }) => {
          const code = (node as HastNode | undefined)?.children?.find((c) => c.tagName === "code");
          // remark adds one trailing newline to a fenced block's text; the block shows without it.
          const body = textOf(code).replace(/\n$/, "");
          return <CodeBlock code={body} fence={fenceOf(code)} streaming={streaming} />;
        },
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
