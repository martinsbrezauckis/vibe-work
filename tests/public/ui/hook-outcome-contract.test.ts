/** Shared history vectors keep the visible hook outcomes and export step counts equivalent across Vibe versions. */
import { expect, it } from "vitest";
import vectors from "../fixtures/hook-outcomes.json";
import { buildRows } from "../../../ui/src/features/transcript/model/build";

it.each(vectors)("hook outcome contract: $name", (vector) => {
  const rows = buildRows(vector.history, []);
  expect(
    rows.flatMap((row) =>
      row.kind === "hook" ? [{ id: row.entry["id"], text: row.text }] : [],
    ),
  ).toEqual(vector.lines);
  expect(
    rows.flatMap((row) =>
      row.kind === "steps"
        ? row.items.flatMap((item) =>
            item.kind === "effect" ? [item.entry["id"]] : [],
          )
        : [],
    ),
  ).toEqual(vector.effects);
});
