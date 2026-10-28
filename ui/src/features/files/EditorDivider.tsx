/** The shared keyboard and pointer divider for paired editors and comparisons; uses the workspace ratio. */
import { useRef, type RefObject } from "react";
import type { Files } from "../../state/files";
import { useViewportWidth } from "../shell/hooks";
export function EditorDivider({
  files,
  sessionId,
  ratio,
  container,
}: {
  files: Files;
  sessionId: string;
  ratio: number;
  container: RefObject<HTMLDivElement | null>;
}) {
  const dragging = useRef(false);
  const narrow = useViewportWidth() <= 640;
  return (
    <div
      className="editor-divider"
      role="separator"
      aria-label="Resize editors"
      aria-orientation={narrow ? "horizontal" : "vertical"}
      aria-valuenow={Math.round(ratio * 100)}
      aria-valuemin={25}
      aria-valuemax={75}
      tabIndex={0}
      data-testid="files.editor-divider"
      onPointerDown={(e) => {
        dragging.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        const rect = container.current?.getBoundingClientRect();
        if (dragging.current && rect)
          files.workspace.ratio(
            sessionId,
            narrow ? (e.clientY - rect.top) / rect.height : (e.clientX - rect.left) / rect.width,
          );
      }}
      onPointerUp={() => {
        dragging.current = false;
      }}
      onPointerCancel={() => {
        dragging.current = false;
      }}
      onKeyDown={(e) => {
        if ((narrow ? ["ArrowUp", "ArrowDown"] : ["ArrowLeft", "ArrowRight"]).includes(e.key)) {
          e.preventDefault();
          files.workspace.ratio(
            sessionId,
            ratio + (["ArrowLeft", "ArrowUp"].includes(e.key) ? -0.05 : 0.05),
          );
        }
      }}
    />
  );
}
