/** The chat header and the side panel's tab bar take their height from one shared token, so their lines meet. */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, it } from "vitest";

const styles = resolve(import.meta.dirname, "../../../ui/src/styles");
const css = (file: string) => readFileSync(resolve(styles, file), "utf8");

/** The declarations of the first rule whose selector is exactly `selector`. */
function rule(text: string, selector: string): string {
  const match = new RegExp(
    `(?:^|\\n)${selector.replace(/[.]/g, "\\.")} \\{([^}]*)\\}`,
  ).exec(text);
  if (match?.[1] === undefined) throw new Error(`no rule ${selector}`);
  return match[1];
}

it("defines the header height once, as a token", () => {
  const tokens = css("tokens.css");
  expect(tokens.match(/--header-height:/g)).toHaveLength(1);
  expect(tokens).toMatch(/--header-height: \d+px;/);
});

it("declares the shared header-height token on both header rules", () => {
  const chat = rule(css("content.css"), ".shead");
  const panel = rule(css("panel.css"), ".ptabs");
  for (const declarations of [chat, panel]) {
    expect(declarations).toContain("min-height: var(--header-height);");
    expect(declarations).not.toMatch(/(?:^|\s)height:/);
  }
});

it("uses the same tab height across every strip", () => {
  const tab = rule(css("chips-tabs.css"), ".ui-tab");
  expect(tab).toContain("min-height: var(--control-height-small);");
});
