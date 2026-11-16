/**
 * The Debug API's `ui/<name>` routes read their body through one of two shared parsers (state/uiControlBody), and
 * no feature route file keeps its own copy.
 *
 * PREDICT: before the shared parsers, 23 feature route files each defined a local `asObject(body)` (17 reading an
 * absent body as `{}`, 6 refusing it), so the guard below lists them and fails; after the migration it passes, and
 * the contract cases pin both behaviours, including the shared refusal sentence.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import {
  bodyObject,
  requiredBodyObject,
} from "../../../ui/src/state/uiControlBody";
import { UiControlError } from "../../../ui/src/state/uiControl";

const FEATURES = join(__dirname, "../../../ui/src/features");

function refusal(parse: () => unknown): { code: string; message: string } {
  try {
    parse();
  } catch (error) {
    expect(error).toBeInstanceOf(UiControlError);
    const { code, message } = error as UiControlError;
    return { code, message };
  }
  throw new Error("the body was accepted");
}

const NOT_AN_OBJECT = {
  code: "invalidParams",
  message: "the body must be a JSON object",
};

describe("route body parsers", () => {
  it("bodyObject reads an absent body as an empty object", () => {
    expect(bodyObject(undefined)).toEqual({});
    expect(bodyObject(null)).toEqual({});
    const body = { action: "open" };
    expect(bodyObject(body)).toBe(body);
    for (const wrong of [[], "open", 3, true])
      expect(refusal(() => bodyObject(wrong))).toEqual(NOT_AN_OBJECT);
  });

  it("requiredBodyObject refuses an absent body like any other non-object", () => {
    const body = { sessionId: "s1" };
    expect(requiredBodyObject(body)).toBe(body);
    for (const wrong of [undefined, null, [], "s1", 0]) {
      expect(refusal(() => requiredBodyObject(wrong))).toEqual(NOT_AN_OBJECT);
    }
  });

  it("no feature route file keeps its own body parser", () => {
    const copies: string[] = [];
    const walk = (dir: string): void => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) walk(path);
        else if (
          /\.tsx?$/.test(name) &&
          /function asObject\(body: unknown\)/.test(readFileSync(path, "utf8"))
        ) {
          copies.push(relative(FEATURES, path));
        }
      }
    };
    walk(FEATURES);
    expect(copies).toEqual([]);
  });
});
