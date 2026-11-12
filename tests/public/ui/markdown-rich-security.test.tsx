/** Hostile renderer output and input corpus. Real renderers stay passive, bounded, offline and source-preserving. */
import { configure, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Markdown } from "../../../ui/src/components/markdown/Markdown";
import { passiveMarkup } from "../../../ui/src/components/markdown/passiveMarkup";

configure({ asyncUtilTimeout: 3000 });

function passive(container: HTMLElement): void {
  expect(
    container.querySelector(
      "script,style,link,img,image,foreignObject,iframe,object,a,use,animate,set,[style],[href],[src]",
    ),
  ).toBeNull();
  for (const node of container.querySelectorAll("*")) {
    expect([...node.attributes].some((attr) => /^on/i.test(attr.name))).toBe(
      false,
    );
  }
}

describe("passive renderer boundary", () => {
  it("removes active tags, styles, external references, handlers and arbitrary attributes before building DOM", () => {
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)" style="background:url(https://evil.invalid)">
      <style>@import url(https://evil.invalid/font.css);</style><script>alert(1)</script>
      <foreignObject><img src="https://evil.invalid/pixel" /></foreignObject>
      <a href="https://evil.invalid"><text>Link</text></a><image href="https://evil.invalid/image" />
      <use href="https://evil.invalid/a.svg#b" /><animate attributeName="href" to="https://evil.invalid" />
      <set attributeName="onload" to="alert(1)" />
      <path d="M0 0 L10 10" fill="url(https://evil.invalid/x)" onclick="alert(1)" data-evil="x" />
      <text>Safe text</text></svg>`;
    const view = render(<>{passiveMarkup(svg, "svg", "test", "source")}</>);
    passive(view.container);
    expect(
      view.container.querySelector("path")?.getAttribute("fill"),
    ).toBeNull();
    expect(view.container.querySelector("[data-evil]")).toBeNull();
    expect(view.container).toHaveTextContent("Safe text");
  });

  it("scopes local arrow references and refuses DTDs and deeply nested XML", () => {
    const svg =
      '<svg><defs><marker id="head" /></defs><path marker-end="url(#head)" /></svg>';
    const view = render(
      <>
        {passiveMarkup(svg, "svg", "first", "one")}
        {passiveMarkup(svg, "svg", "second", "two")}
      </>,
    );
    expect(view.container.querySelectorAll("marker")[0]).toHaveAttribute(
      "id",
      "first-head",
    );
    expect(view.container.querySelectorAll("path")[1]).toHaveAttribute(
      "marker-end",
      "url(#second-head)",
    );
    expect(() =>
      passiveMarkup(
        '<!DOCTYPE svg [<!ENTITY x "test">]><svg>&x;</svg>',
        "svg",
        "x",
        "x",
      ),
    ).toThrow();
    expect(() =>
      passiveMarkup(
        `<svg>${"<g>".repeat(100)}${"</g>".repeat(100)}</svg>`,
        "svg",
        "x",
        "x",
      ),
    ).toThrow();
  });
});

describe("untrusted diagram source", () => {
  it.each([
    'graph TD\nA[Start]\nclick A "https://evil.invalid"',
    '%%{init: {"securityLevel":"loose","htmlLabels":true}}%%\ngraph TD\nA --> B',
    "graph TD\nA --> B\nstyle A fill:url(https://evil.invalid/x)",
    `graph TD\n${"A --> B\n".repeat(200)}`,
  ])(
    "falls back for unsafe directives and oversized diagrams",
    async (source) => {
      const view = render(
        <Markdown source={`\`\`\`mermaid\n${source}\n\`\`\``} />,
      );
      await screen.findByText("Could not draw this diagram.");
      expect(view.container.querySelector("pre code")?.textContent).toBe(
        source,
      );
      passive(view.container);
    },
  );

  it("shows HTML-looking labels as plain drawing text", async () => {
    const source =
      'graph TD\nA["<img src=https://evil.invalid onerror=alert(1)>"] --> B[Done]';
    const view = render(
      <Markdown source={`\`\`\`mermaid\n${source}\n\`\`\``} />,
    );
    await screen.findByRole("img", { name: /graph TD/ });
    passive(view.container);
  });
});

describe("untrusted math source", () => {
  it.each([
    String.raw`\href{https://evil.invalid}{click}`,
    String.raw`\url{https://evil.invalid}`,
    String.raw`\includegraphics{https://evil.invalid/pixel}`,
    String.raw`\htmlStyle{background:url(https://evil.invalid)}{x}`,
    String.raw`\htmlClass{evil}{x}`,
    String.raw`\def\x{\x}\x`,
  ])(
    "does not activate resource, HTML or recursive commands: %s",
    async (source) => {
      const view = render(<Markdown source={`$${source}$`} />);
      await waitFor(() => {
        expect(view.container.querySelector(".md-math")).not.toBeNull();
      });
      // Wait for a following valid formula to settle too, exercising the same lazy renderer chunk.
      view.rerender(<Markdown source={`$${source}$ and $x^2$`} />);
      await waitFor(() => {
        expect(view.container.querySelector("math")).not.toBeNull();
      });
      passive(view.container);
    },
  );
});
