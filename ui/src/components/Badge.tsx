/** Shared passive and actionable chips, retaining the established presentations. Features supply content and state; shared CSS owns the visuals. */
import { createElement, type ComponentProps, type HTMLAttributes, type ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

const PRESENTATIONS = {
  metadata: "ui-badge",
  "page-check": "pages-visual-badge",
  session: "tag",
  "cli-session": "tag cli-tag",
  pin: "tag pin",
  folder: "tag folder-chip",
  temporary: "tag temp-chip",
  "project-label": "pj-tag",
  "project-session": "pj-chip",
  "connector-status": "cstat",
  "market-status": "cstat",
  context: "ctxchip",
  "document-context": "ctxchip docctx",
  notice: "notice-pill",
  "page-label": "pages-tag",
  "page-state": "pages-pill",
  "question-label": "q-tag",
  "user-label": "u-tag",
  "loop-label": "u-tag u-loop",
  "voice-label": "u-tag u-voice",
  "context-label": "ctx-chip",
  "image-label": "img-chip",
  sources: "copy-btn sources-chip",
  "voice-strip": "vchat-tag",
  "pull-request": "chip pr-chip",
  worktree: "chip wt-chip",
} as const;
type Content = {
  presentation?: keyof typeof PRESENTATIONS;
  tone?: "neutral" | "ok" | "warn" | "bad";
  state?: string;
  icon?: IconName;
  children?: ReactNode;
  "data-tone"?: string;
};
type BadgeProps = Content &
  (
    | (Omit<HTMLAttributes<HTMLSpanElement>, "className"> & { as?: "span" })
    | (Omit<ComponentProps<"button">, "className"> & { as: "button"; "data-testid": string })
    | (Omit<HTMLAttributes<HTMLDivElement>, "className"> & { as: "div" })
  );

/** One chip primitive; a native button only when it performs an action. */
export function Badge({
  presentation = "metadata",
  tone = "neutral",
  state,
  icon,
  children,
  ...props
}: BadgeProps) {
  const suffix =
    state === undefined
      ? ""
      : presentation === "pull-request"
        ? ` pr-${state}`
        : presentation === "worktree"
          ? ` wt-${state}`
          : presentation === "market-status"
            ? ` mstat-${state}`
            : ` ${state}`;
  const shared = {
    className:
      PRESENTATIONS[presentation] +
      suffix +
      (presentation === "pull-request" && tone === "bad" ? " bad" : ""),
    "data-size": presentation === "metadata" ? "small" : undefined,
    "data-tone": props["data-tone"] ?? tone,
  };
  const content = (
    <>
      {icon && <Icon name={icon} size={12} />}
      {children}
    </>
  );
  if (props.as === "button") {
    const { as, ...button } = props;
    return createElement(
      as,
      { type: "button", ...button, ...shared, "data-testid": button["data-testid"] },
      content,
    );
  }
  if (props.as === "div") {
    const { as, ...div } = props;
    return createElement(as, { ...div, ...shared }, content);
  }
  const { as = "span", ...span } = props;
  return createElement(as, { ...span, ...shared }, content);
}
