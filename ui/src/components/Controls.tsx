/**
 * Shared app actions, metadata badges and list disclosures. Depends on React and Icon;
 * feature pages supply intent and content while shared CSS owns type, sizing and states.
 */
import type { ComponentProps, HTMLAttributes } from "react";

type ButtonProps = Omit<ComponentProps<"button">, "className"> & {
  "data-testid": string;
};

/** An immediate on/off action with the shared track and native disabled behavior. */
export function Switch(props: Omit<ButtonProps, "role"> & { "aria-checked": boolean }) {
  return (
    <button
      type="button"
      {...props}
      data-testid={props["data-testid"]}
      className="switch"
      role="switch"
    />
  );
}

/** A label/detail column beside an immediate setting control, with no extra wrapper. */
export function SettingsRow(props: Omit<HTMLAttributes<HTMLDivElement>, "className">) {
  return <div {...props} className="check" />;
}

const BUTTON_APPEARANCE = {
  expander: "show-all-btn",
  compact: "mini",
  page: "pages-btn",
  preview: "preview-btn",
  project: "pj-btn",
  choice: "cbtn",
  dialog: "btn-primary",
  setup: "btn-secondary",
  "key-link": "key-link",
  "text-link": "link-btn",
  link: "ext-link",
  "browser-menu": "browser-menu-item",
  icon: "ibtn",
  status: "sb-vault",
  "file-reference": "fref",
} as const;
const BUTTON_PLACEMENT = {
  stop: "stop",
  "detail-close": "cdetail-close",
  "pr-title": "pr-title",
  "browser-stop": "browser-stop",
  "browser-notes": "browser-notes-send",
  "browser-capture": "preview-capture-button",
  "browser-save": "preview-save-more",
  "vault-actions": "vault-actions-trigger",
  "sources-close": "sources-close",
} as const;

/**
 * Shared actions retain their existing presentation while callers supply intent and geometry. `shape` names the
 * plain button's two shapes: "ghost" (default; borderless, the toolbar button) and "outlined" (1 px border, 30 px
 * high: the Settings, dialog and form button).
 */
export function Button({
  variant,
  shape = "ghost",
  appearance,
  placement,
  ...props
}: ButtonProps & {
  variant?: "primary" | "danger" | undefined;
  shape?: "ghost" | "outlined";
  appearance?: keyof typeof BUTTON_APPEARANCE | undefined;
  placement?: keyof typeof BUTTON_PLACEMENT;
}) {
  const small = shape === "outlined";
  const base = appearance === undefined ? "tb-btn" : BUTTON_APPEARANCE[appearance];
  const emphasis =
    variant === "primary" && (appearance === "choice" || appearance === "page") ? "go" : variant;
  return (
    <button
      type="button"
      {...props}
      data-testid={props["data-testid"]}
      className={[
        base,
        emphasis,
        appearance === "project" && small ? "small" : undefined,
        placement && BUTTON_PLACEMENT[placement],
      ]
        .filter(Boolean)
        .join(" ")}
      data-shape={shape}
      data-size={small ? "small" : "normal"}
    />
  );
}

export { IconButton } from "./IconButton";

/** Navigation links and destructive list actions that open a confirmation. */
export function LinkButton(props: ButtonProps & { variant?: "danger" }) {
  return <Button {...props} appearance="link" data-testid={props["data-testid"]} />;
}

export { Badge } from "./Badge";

/** A list-row disclosure; feature CSS may lay out its contents, never restyle the control. */
export function RowDisclosure(props: ButtonProps) {
  return (
    <button type="button" {...props} data-testid={props["data-testid"]} className="ui-disclosure" />
  );
}
