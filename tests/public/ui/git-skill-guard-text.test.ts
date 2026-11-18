/** The built-in Git skill must give the same next step as the remote-write guard. */
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("directs a refused push to the live Push or Create PR controls", () => {
  const skill = readFileSync("../app/plugin/vw/skills/git/SKILL.md", "utf8");
  const guard = readFileSync("../crates/vw-gh-broker/src/lib.rs", "utf8");
  const nextStep =
    "For remote changes, ask the user to use Push or Create PR in the folder's Changes tab, or make the change on the code hosting site.";
  expect(guard).toContain(nextStep);
  expect(skill).toContain(nextStep);
  expect(skill).not.toContain("The Push button is in the Changes tab.");
});
