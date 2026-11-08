/** Derived Account row text, used by Settings; metadata only. */
import type { MistralKeyTestView } from "../protocol/core";
import type { KeyUi } from "./keyStore";
import { describeProblem } from "./keyText";

/** Restore the selected credential's authentication warning without making another network request. */
export function recordedKeyTest(test: MistralKeyTestView | null | undefined): KeyUi["test"] {
  const reason = test?.code === 401 ? "unauthorized" : test?.code === 403 ? "forbidden" : null;
  if (reason === null) return { phase: "idle" };
  return {
    phase: "failed",
    reason,
    text: describeProblem({
      code: "keyService",
      serverCode: reason,
      message: "",
      method: null,
      summary: null,
    }),
  };
}
/** A saved test never includes the response body. */
export function savedTestText(test: MistralKeyTestView | null): string {
  if (test === null) return "Not tested yet";
  if (test.modelCount !== null) return "Key works";
  if (test.code !== null) return `Mistral answered ${String(test.code)}: ${test.text ?? ""}`;
  return test.text ?? "Not tested yet";
}
