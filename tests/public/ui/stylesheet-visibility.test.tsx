/** Visibility assertions use the app's real styles; synthetic rectangles do not claim browser layout. */
import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { Button } from "../../../ui/src/components/Controls";

it("shows the shared action with its real stylesheet", () => {
  render(<Button data-testid="visibility-probe">Save</Button>);
  expect(screen.getByTestId("visibility-probe")).toBeVisible();
});
