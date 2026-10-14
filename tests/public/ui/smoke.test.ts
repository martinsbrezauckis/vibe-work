/**
 * Public UI smoke test: the files the app needs to start are present and carry the design tokens.
 * Imports nothing but Node built-ins (vitest globals are enabled), so it runs from any checkout.
 */
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const uiRoot = process.cwd();

describe("ui scaffold", () => {
  it("has an entry page and a token sheet defining the base colour", () => {
    expect(existsSync(resolve(uiRoot, "index.html"))).toBe(true);
    const tokens = readFileSync(
      resolve(uiRoot, "src/styles/tokens.css"),
      "utf8",
    );
    expect(tokens).toContain("--bg:");
  });
});
