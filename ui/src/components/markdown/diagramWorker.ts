/** Dedicated DOM-free diagram renderer. Called only by diagramWorkerClient through a bundled worker. */
import { drawDiagram } from "./diagramRender";
import type { DiagramWorkerResult } from "./diagramWorkerMessages";

self.onmessage = (event: MessageEvent<string>): void => {
  void drawDiagram(event.data).then(
    (svg) => {
      const result: DiagramWorkerResult = { ok: true, svg };
      self.postMessage(result);
    },
    (error: unknown) => {
      const result: DiagramWorkerResult = {
        ok: false,
        error:
          error instanceof Error
            ? { name: error.name, message: error.message }
            : { name: "Error", message: "Could not draw this diagram" },
      };
      self.postMessage(result);
    },
  );
};
