/**
 * A path on one line with a copy action, shared by plugin cards and the copy end dialog. The path itself is the shared
 * `PathText`: when it does not fit it loses its start, so the folder's own name stays readable, and the full path is the
 * tooltip. The copy action copies the original text.
 * Depends on: PathText, IconButton, styles/shared-controls.css (`.path-line`). Called by: LocalPlugins, EndDialog.
 */
import { writeClipboard } from "../state/clipboard";
import { IconButton } from "./IconButton";
import { PathText } from "./PathText";

/** Shows the path start-truncated with a full tooltip, and copies its original text. */
export function PathLine({ path, className = "" }: { path: string; className?: string }) {
  return (
    <div className={`path-line mono ${className}`}>
      <PathText path={path} />
      <IconButton
        data-testid="path-line.copy"
        icon="copy"
        label="Copy path"
        onClick={() => void writeClipboard(path).catch(() => undefined)}
      />
    </div>
  );
}
