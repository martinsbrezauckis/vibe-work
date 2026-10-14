/**
 * Shared app actions, metadata badges and list disclosures. Depends on React and Icon;
 * feature pages supply intent and content while shared CSS owns type, sizing and states.
 */
import type { ButtonHTMLAttributes, HTMLAttributes } from "react";
import { Icon, type IconName } from "./Icon";

type ButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className">;

/** A secondary action by default, with the app's primary and danger variants. */
export function Button({
  variant,
  small = false,
  ...props
}: ButtonProps & {
  variant?: "primary" | "danger";
  small?: boolean;
}) {
  return (
    <button
      type="button"
      {...props}
      className={`tb-btn${variant ? ` ${variant}` : ""}`}
      data-size={small ? "small" : "normal"}
    />
  );
}

/** A named icon action; locked state has a shared visible indicator. */
export function IconButton({
  icon,
  state,
  children,
  ...props
}: ButtonProps & {
  icon: IconName;
  state?: "locked" | "unlocked";
}) {
  return (
    <button type="button" {...props} className="ibtn" data-state={state} data-icon={icon}>
      <Icon key={icon} name={icon} size={16} />
      {children}
    </button>
  );
}

/** Navigation within the app uses the same link treatment as outbound Settings links. */
export function LinkButton(props: ButtonProps) {
  return <button type="button" {...props} className="ext-link" />;
}

/** Passive metadata; the same small type scale as shared selector text. */
export function Badge(props: Omit<HTMLAttributes<HTMLSpanElement>, "className">) {
  return <span {...props} className="ui-badge" data-size="small" />;
}

/** A list-row disclosure; feature CSS may lay out its contents, never restyle the control. */
export function RowDisclosure(props: ButtonProps) {
  return <button type="button" {...props} className="ui-disclosure" />;
}
