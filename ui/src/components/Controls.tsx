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

/**
 * One look per job (sticker:gap-buttons, sticker:button-presets). Besides the plain button (`shape` ghost or outlined)
 * and `variant="primary"`, an appearance names a job with its own size or role: a compact action, a link, a menu
 * item, an icon, the status-bar vault control and a file reference.
 */
const BUTTON_APPEARANCE = {
  compact: "tb-btn mini",
  link: "ext-link",
  menu: "menu-item",
  icon: "ibtn",
  status: "sb-vault",
  "file-reference": "fref",
} as const;
/** Layout hooks only: feature CSS may place these buttons, never restyle them. */
const BUTTON_PLACEMENT = {
  page: "pages-btn",
  preview: "preview-btn",
  project: "pj-btn",
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
 * Shared actions use one visual style per job; placement names add layout hooks only. `shape` names the plain
 * button's two shapes: "ghost" (default; borderless, the toolbar button) and "outlined" (1 px border, 30 px high: the
 * Settings, dialog and form button). `variant="primary"` is the one filled action of a view or dialog.
 */
export function Button({
  variant,
  shape,
  appearance,
  placement,
  ...props
}: ButtonProps & {
  variant?: "primary" | "danger" | undefined;
  shape?: "ghost" | "outlined";
  appearance?: keyof typeof BUTTON_APPEARANCE | undefined;
  placement?: keyof typeof BUTTON_PLACEMENT | undefined;
}) {
  const base = appearance === undefined ? "tb-btn" : BUTTON_APPEARANCE[appearance];
  const resolvedShape = shape ?? (appearance === undefined ? "ghost" : "outlined");
  return (
    <button
      type="button"
      {...props}
      data-testid={props["data-testid"]}
      className={[base, variant, placement && BUTTON_PLACEMENT[placement]]
        .filter(Boolean)
        .join(" ")}
      data-shape={resolvedShape}
      data-size="small"
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
