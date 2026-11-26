/** jsdom has no Worker. This adapter preserves renderer mocks; real browser tests cover thread isolation. */
import type { DiagramWorkerResult } from "../../../../ui/src/components/markdown/diagramWorkerMessages";

class TestDiagramWorker extends EventTarget {
  onmessage: ((event: MessageEvent<DiagramWorkerResult>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  onmessageerror: ((event: MessageEvent<unknown>) => void) | null = null;
  private closed = false;

  postMessage(source: unknown): void {
    void import("../../../../ui/src/components/markdown/diagramRender")
      .then(({ drawDiagram }) => {
        if (typeof source !== "string")
          throw new Error("Expected diagram source");
        return drawDiagram(source);
      })
      .then(
        (svg) => {
          this.reply({ ok: true, svg });
        },
        (error: unknown) => {
          this.reply({
            ok: false,
            error:
              error instanceof Error
                ? { name: error.name, message: error.message }
                : { name: "Error", message: "Could not draw this diagram" },
          });
        },
      );
  }

  terminate(): void {
    this.closed = true;
  }

  private reply(data: DiagramWorkerResult): void {
    if (!this.closed) this.onmessage?.(new MessageEvent("message", { data }));
  }
}

if (typeof Worker === "undefined") globalThis.Worker = TestDiagramWorker;
