/**
 * A folder path that, when it does not fit, loses its START, not its end: the folder's own name
 * is what the reader needs, so `/home/me/work/projects/weather-app` becomes `…ork/projects/weather-app`. The full
 * path is the tooltip. One component for the status bar, the new-chat folder row and the project card footer.
 * How: the box runs right-to-left so the browser's ellipsis lands on the left, and the text sits in an isolated
 * left-to-right run, so the characters (and a Windows `C:\...` or UNC `\\server\share` path) keep their order.
 * Depends on: styles/shell.css (`.path-start`). Called by: StatusBar, NewChatPanel, ProjectCardView.
 * Invariants: the text content is the whole path (copy, search and screen readers get it); nothing is cut in the
 * data, only in the painting.
 */
import type { ReactElement } from "react";

/** The path, start-truncated by the browser when the box is narrower than the text. */
export function PathText({
  path,
  className,
  testId,
  title,
}: {
  path: string;
  /** The caller's own classes (size, colour, flex rules); `path-start` is added here. */
  className?: string;
  testId?: string;
  /** Overrides the tooltip (default: the full path). */
  title?: string;
}): ReactElement {
  return (
    <span
      className={className === undefined ? "path-start" : `path-start ${className}`}
      title={title ?? path}
      {...(testId === undefined ? {} : { "data-testid": testId })}
    >
      <bdi dir="ltr">{path}</bdi>
    </span>
  );
}
