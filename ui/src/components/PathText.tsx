/**
 * A folder path that, when it does not fit, loses its START, not its end: the folder's own name
 * is what the reader needs, so `/home/me/work/projects/weather-app` becomes `…ork/projects/weather-app`. The full
 * path is the tooltip. One component for the status bar, the new-chat folder row and the project card footer.
 * How: the tail runs right-to-left with its own leading ellipsis when measured text does not fit.
 * The path sits in an isolated left-to-right run, so the characters (and a Windows `C:\...` or UNC `\\server\share` path) keep their order.
 * Depends on: styles/shell.css (`.path-start`). Called by: StatusBar, NewChatPanel, ProjectCardView.
 * Invariants: only the actual home prefix is abbreviated to ~; the tooltip keeps the full path.
 * Truncation is visual, and preserves the folder name at the end.
 */
import {
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  type ReactElement,
} from "react";

/** The actual home folder reported by the core; only that prefix is abbreviated. */
export const PathHome = createContext<string | null>(null);

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
  const home = useContext(PathHome)?.replace(/[\\/]+$/, "");
  const display =
    home && (path === home || path.startsWith(`${home}/`) || path.startsWith(`${home}\\`))
      ? `~${path.slice(home.length)}`
      : path;
  const frame = useRef<HTMLSpanElement>(null);
  const text = useRef<HTMLElement>(null);
  const [cut, setCut] = useState(false);
  useLayoutEffect(() => {
    const measure = () => {
      const box = frame.current;
      const full = text.current;
      if (!box || !full || box.clientWidth <= 0) {
        setCut(false);
        return;
      }
      const style = getComputedStyle(box);
      const padding =
        (Number.parseFloat(style.paddingLeft) || 0) + (Number.parseFloat(style.paddingRight) || 0);
      setCut(full.scrollWidth > box.clientWidth - padding);
    };
    measure();
    const observer = new ResizeObserver(measure);
    if (frame.current) observer.observe(frame.current);
    if (text.current) observer.observe(text.current);
    return () => {
      observer.disconnect();
    };
  }, [display]);
  return (
    <span
      ref={frame}
      dir="ltr"
      className={className === undefined ? "path-start" : `path-start ${className}`}
      title={title ?? path}
      {...(testId === undefined ? {} : { "data-testid": testId })}
    >
      {cut && <span aria-hidden="true">…</span>}
      <span className="path-tail" dir="rtl">
        <bdi ref={text} dir="ltr">
          {display}
        </bdi>
      </span>
    </span>
  );
}
