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

it("gives the chat header and the side panel tab bar that same height", () => {
  const chat = rule(css("content.css"), ".shead");
  const panel = rule(css("panel.css"), ".ptabs");
  for (const declarations of [chat, panel]) {
    expect(declarations).toContain("min-height: var(--header-height);");
    expect(declarations).not.toMatch(/(?:^|\s)height:/);
  }
});

it("sizes the selected tab's pill from the tab, not from fixed offsets", () => {
  const pill = rule(css("feel.css"), ".ptab-pill");
  expect(pill).not.toMatch(/bottom:/);
});
