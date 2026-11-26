/** Window-side diagram deadline and worker lifecycle. MermaidBlock alone converts the returned SVG to React. */
import { isDiagramWorkerResult } from "./diagramWorkerMessages";
import { DiagramInputError, DiagramLimitError, DiagramUnsupportedError } from "./mermaid/types";

const DRAW_TIMEOUT_MS = 2000;

/** Each drawing owns a worker so a stalled engine cannot block another chat or the window's timer. */
export function drawDiagramInWorker(source: string, signal?: AbortSignal): Promise<string> {
  return new Promise((resolve, reject) => {
    const aborted = (): DOMException => new DOMException("Diagram drawing cancelled", "AbortError");
    if (signal?.aborted) {
      reject(aborted());
      return;
    }
    let worker: Worker;
    try {
      worker = new Worker(new URL("./diagramWorker.ts", import.meta.url), { type: "module" });
    } catch (error) {
      reject(error instanceof Error ? error : new Error("Diagram worker failed"));
      return;
    }
    let settled = false;
    const cleanup = (): boolean => {
      if (settled) return false;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", cancel);
      worker.onmessage = null;
      worker.onerror = null;
      worker.onmessageerror = null;
      worker.terminate();
      return true;
    };
    const fail = (error: Error): void => {
      if (cleanup()) reject(error);
    };
    const cancel = (): void => {
      fail(aborted());
    };
    const timer = setTimeout(() => {
      fail(new DiagramLimitError("Diagram took too long to draw"));
    }, DRAW_TIMEOUT_MS);
    signal?.addEventListener("abort", cancel, { once: true });
    worker.onmessage = (event: MessageEvent<unknown>): void => {
      const result = event.data;
      if (!isDiagramWorkerResult(result)) {
        fail(new Error("Invalid diagram worker response"));
      } else if (result.ok) {
        if (cleanup()) resolve(result.svg);
      } else {
        const { name, message } = result.error;
        const error =
          name === "DiagramInputError"
            ? new DiagramInputError(message)
            : name === "DiagramLimitError"
              ? new DiagramLimitError(message)
              : name === "DiagramUnsupportedError"
                ? new DiagramUnsupportedError(message)
                : new Error(message);
        error.name = name;
        fail(error);
      }
    };
    worker.onerror = (): void => {
      fail(new Error("Diagram worker failed"));
    };
    worker.onmessageerror = (): void => {
      fail(new Error("Could not read diagram worker response"));
    };
    try {
      worker.postMessage(source);
    } catch (error) {
      fail(error instanceof Error ? error : new Error("Could not send diagram to worker"));
    }
  });
}
