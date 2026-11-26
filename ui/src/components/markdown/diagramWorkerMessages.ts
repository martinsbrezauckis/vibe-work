/** Structured-clone protocol shared by the diagram worker and its window-side client. */
export type DiagramWorkerResult =
  { ok: true; svg: string } | { ok: false; error: { name: string; message: string } };

/** Reject malformed messages rather than passing unexpected markup to the passive SVG gate. */
export function isDiagramWorkerResult(value: unknown): value is DiagramWorkerResult {
  if (typeof value !== "object" || value === null || !("ok" in value)) return false;
  if (value.ok === true) return "svg" in value && typeof value.svg === "string";
  if (value.ok !== false || !("error" in value)) return false;
  const error = value.error;
  return (
    typeof error === "object" &&
    error !== null &&
    "name" in error &&
    typeof error.name === "string" &&
    "message" in error &&
    typeof error.message === "string"
  );
}
