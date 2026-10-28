/** Shared project/connector card frame with the existing presentation variants. Features own content and actions; shared CSS owns the frame and selection states. */
import type { HTMLAttributes } from "react";

/** A card's existing selected and interactive state, with no extra wrapper. */
export function Card({
  variant = "project",
  selected = false,
  interactive,
  ...props
}: Omit<HTMLAttributes<HTMLElement>, "className"> & {
  variant?: "project" | "connector";
  selected?: boolean;
  interactive?: boolean;
}) {
  return variant === "connector" ? (
    <div {...props} className={`ctile ${selected ? "sel" : ""}`} data-interactive={interactive} />
  ) : (
    <article
      {...props}
      className="pj-card"
      data-selected={selected}
      data-interactive={interactive}
    />
  );
}
