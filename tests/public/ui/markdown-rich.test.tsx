/** Rich chat Markdown acceptance tests. Uses real renderers; source and hostile input stay inert. */
import {
  configure,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Markdown } from "../../../ui/src/components/markdown/Markdown";

const diagram = "graph LR\n  A[Draft] --> B[Review] --> C[Done]";
configure({ asyncUtilTimeout: 3000 });

describe("diagrams in Markdown", () => {
  it.each([
    "sequenceDiagram\nAlice->>Bob: Hello\nBob-->>Alice: Ready",
    "stateDiagram-v2\n[*] --> Ready\nReady --> Done\nDone --> [*]",
    "classDiagram\nAnimal <|-- Dog\nAnimal : +name",
    "erDiagram\nCUSTOMER ||--o{ ORDER : places",
  ])("draws the supported Mermaid diagram families: %s", async (source) => {
    const view = render(
      <Markdown source={`\`\`\`mermaid\n${source}\n\`\`\``} />,
    );
    await waitFor(() => {
      expect(view.container.querySelector(".md-diagram svg")).not.toBeNull();
    });
  });
  it("draws a Mermaid fence with its source as the accessible alternative and copy value", async () => {
    const copy = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: copy },
    });
    const { container } = render(
      <Markdown source={`\`\`\`mermaid\n${diagram}\n\`\`\``} />,
    );
    const drawing = await screen.findByRole("img", { name: /graph LR/ });
    expect(drawing).toHaveAttribute("aria-label", diagram);
    expect(drawing.querySelector("svg")).not.toBeNull();
    expect(drawing).toHaveTextContent("Draft");
    fireEvent.click(
      screen.getByRole("button", { name: "Copy diagram source" }),
    );
    await waitFor(() => {
      expect(copy).toHaveBeenCalledWith(diagram);
    });
    expect(
      container.querySelector("style, script, foreignObject, a, image, use"),
    ).toBeNull();
    expect(container.querySelector("[style]")).toBeNull();
  });

  it("keeps an invalid diagram as normal code and explains the failure", async () => {
    const source = "this is not a diagram";
    const { container } = render(
      <Markdown source={`\`\`\`mermaid\n${source}\n\`\`\``} />,
    );
    await screen.findByText("Could not draw this diagram.");
    expect(container.querySelector("pre code")?.textContent).toBe(source);
    expect(
      screen.getByRole("button", { name: "Copy code" }),
    ).toBeInTheDocument();
  });

  it.each(["graph TD\nA[unfinished", "graph TD\nA -->"])(
    "keeps malformed graph syntax as source: %s",
    async (source) => {
      const view = render(
        <Markdown source={`\`\`\`mermaid\n${source}\n\`\`\``} />,
      );
      await screen.findByText("Could not draw this diagram.");
      expect(view.container.querySelector("pre code")?.textContent).toBe(
        source,
      );
    },
  );

  it("draws semicolon-separated graph statements without splitting quoted labels", async () => {
    const source = 'graph LR; A["Draft; review"] --> B[Done];';
    const view = render(
      <Markdown source={`\`\`\`mermaid\n${source}\n\`\`\``} />,
    );
    await waitFor(() => {
      expect(view.container.querySelector(".md-diagram svg")).not.toBeNull();
    });
    expect(view.container.querySelector("svg")).toHaveTextContent(
      "Draft; review",
    );
  });

  it("waits until streaming finishes before drawing", async () => {
    const source = `\`\`\`mermaid\n${diagram}\n\`\`\``;
    const view = render(<Markdown source={source} streaming />);
    expect(view.container.querySelector("pre code")?.textContent).toBe(diagram);
    expect(view.container.querySelector("svg")).toBeNull();
    view.rerender(<Markdown source={source} />);
    await screen.findByRole("img", { name: /graph LR/ });
  });
});

describe("math in Markdown", () => {
  it.each([
    ["$x^2 + y^2$", "inline"],
    [String.raw`\(x_1 + \alpha\)`, "inline"],
    ["$$\\frac{1}{2}$$", "display"],
    ["$$\n\\sum_{i=1}^{n} i\n$$", "display"],
    [String.raw`\[\int_0^1 x^2 dx\]`, "display"],
  ])("renders %s as readable %s math", async (source, mode) => {
    const { container } = render(<Markdown source={source} lineBreaks />);
    await waitFor(() => {
      expect(container.querySelector("math")).not.toBeNull();
    });
    expect(container.querySelector(".md-math")).toHaveAttribute(
      "data-display",
      mode,
    );
    expect(
      container.querySelector("math")?.getAttribute("aria-label"),
    ).toBeTruthy();
    expect(
      container.querySelector("script, style, img, a, [style]"),
    ).toBeNull();
  });

  it("renders nearby formulas while leaving currency and escaped dollars as text", async () => {
    const source = String.raw`Pay $5 and $10, or US$20. Formula $x^2$; escaped \$x\$ stays text.`;
    const { container } = render(<Markdown source={source} />);
    await waitFor(() => {
      expect(container.querySelectorAll("math")).toHaveLength(1);
    });
    expect(container).toHaveTextContent("Pay $5 and $10, or US$20.");
    expect(container).toHaveTextContent("escaped $x$ stays text.");
  });

  it("leaves code, unmatched delimiters and ordinary prices alone", () => {
    const source =
      "`$x$` and `\\(x\\)`\n\n```text\n$$x$$\n```\n\n$5 and $10\n\nUnfinished $x";
    const { container } = render(<Markdown source={source} />);
    expect(container.querySelector("math")).toBeNull();
    expect(container.querySelector("pre code")?.textContent).toBe("$$x$$");
    expect(container).toHaveTextContent("$5 and $10");
  });
});
