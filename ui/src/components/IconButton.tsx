/**
 * Shared compact icon button with optional visible state, using the app's ibtn style and Lucide set.
 * Accessible names and tooltips are required.
 * Depends on: Icon, React button props. Called by: the browser toolbar, its save control and sidebar projects.
 */
import type { ComponentProps, ReactElement } from "react";

import { Icon, type IconName } from "./Icon";

/** A labelled icon action; accepts native button attributes and a ref. */
export function IconButton({
  icon,
  iconSize = 16,
  label,
  text,
  title = label,
  className = "",
  ...props
}: Omit<ComponentProps<"button">, "children" | "aria-label"> & {
  "data-testid": string;
  icon: IconName;
  /** Smaller glyphs for dense row actions; the button retains its own hit area. */
  iconSize?: ComponentProps<typeof Icon>["size"];
  label: string;
  /** Optional current state rendered beside the glyph. */
  text?: string;
}): ReactElement {
  return (
    <button
      type="button"
      {...props}
      data-testid={props["data-testid"]}
      className={`ibtn ${className}`}
      aria-label={label}
      title={title}
    >
      <Icon name={icon} size={iconSize} />
      {text !== undefined && <span>{text}</span>}
    </button>
  );
}
