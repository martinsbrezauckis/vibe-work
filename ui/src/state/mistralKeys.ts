/** Derived Account row text, used by Settings; metadata only. */
import type { MistralKeyTestView } from "../protocol/core";
/** A saved test never includes the response body. */
export function savedTestText(test: MistralKeyTestView | null): string {
  if (test === null) return "Not tested yet";
  if (test.modelCount !== null) return "Key works";
  if (test.code !== null) return `Mistral answered ${String(test.code)}: ${test.text ?? ""}`;
  return test.text ?? "Not tested yet";
}
