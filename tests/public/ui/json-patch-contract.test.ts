/** Shared wire-patch vectors pin Rust/TypeScript replay and failure atomicity; run by the public UI suite. */
import { expect, it } from "vitest";
import vectors from "../fixtures/json-patch.json";
import { applyPatch, PatchError } from "../../../ui/src/state/jsonPatch";

it.each(vectors)("shared patch contract: $name", (vector) => {
  const before = structuredClone(vector.document);
  if (vector.error) {
    expect(() => applyPatch(vector.document, vector.operations)).toThrow(
      PatchError,
    );
  } else {
    expect(applyPatch(vector.document, vector.operations)).toEqual(
      vector.expected,
    );
  }
  expect(vector.document).toEqual(before);
});
