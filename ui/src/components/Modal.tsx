/**
 * The app's modal dialog: the dimmed scrim beside a `role="dialog"` `aria-modal` element that takes the focus.
 * Depends on: react, hooks/useModalFocus. Called by: the feature dialogs (render it only while the dialog is open).
 * Invariants:
 * - The focus rules are `useModalFocus`: the initial focus (`data-dialog-initial-focus="true"`, else the first
 *   control), Tab kept inside, everything outside inert and hidden from screen readers while it shows, focus back
 *   where it was after, and Escape runs `onClose`.
 * - The scrim stays clickable (only `aria-hidden`); a click on it runs `onClose` unless `scrimCloses` is false (a
 *   dialog that must be answered with its buttons, or one that is busy).
 * - The dialog renders where it is placed (no portal), so a dialog keeps the page region and stacking it had.
 * - Every other attribute (test id, `aria-labelledby`, `aria-describedby`, data attributes) goes on the dialog
 *   element; `className` defaults to `modal`.
 */
import { type HTMLAttributes, type ReactElement, type RefObject, useRef } from "react";

import { useModalFocus } from "../hooks/useModalFocus";

/** Props of `Modal`: every dialog attribute except role, `aria-modal` and `tabIndex`, which it sets itself. */
export interface ModalProps extends Omit<
  HTMLAttributes<HTMLDivElement>,
  "role" | "aria-modal" | "tabIndex"
> {
  /** Escape, and a click on the scrim unless `scrimCloses` is false. */
  onClose: () => void;
  /** False: a click on the scrim does nothing. */
  scrimCloses?: boolean;
  /** The scrim's `data-testid`. */
  scrimTestId?: string;
  /** The dialog element, for a caller that measures or queries it. */
  dialogRef?: RefObject<HTMLDivElement | null>;
  /** The dialog's stable test id (`<area>.<dialog>`). */
  "data-testid": string;
}

/** One open modal dialog. */
export function Modal({
  onClose,
  scrimCloses = true,
  scrimTestId,
  dialogRef,
  className = "modal",
  children,
  "data-testid": testId,
  ...dialog
}: ModalProps): ReactElement {
  const own = useRef<HTMLDivElement>(null);
  const ref = dialogRef ?? own;
  useModalFocus(true, ref, onClose);
  return (
    <>
      <div
        data-testid={scrimTestId}
        className="scrim"
        onClick={scrimCloses ? onClose : undefined}
      />
      <div
        {...dialog}
        data-testid={testId}
        ref={ref}
        className={className}
        role="dialog"
        aria-modal="true"
        tabIndex={-1}
      >
        {children}
      </div>
    </>
  );
}
