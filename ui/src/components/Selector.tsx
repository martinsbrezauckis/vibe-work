/**
 * One pick-one selector for Settings and composer pickers: compact trigger, caret and a
 * viewport-bound menu. Depends on React, ReactDOM and Icon; callers own values and side effects.
 * Keyboard arrows/Home/End move, Enter/Space pick, Escape closes and restores trigger focus.
 */
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "./Icon";

/** A value and its optional contextual description, icon or permission dot. */
export interface SelectorOption<T extends string> {
  value: T;
  label: string;
  description?: string;
  tooltip?: string;
  icon?: IconName;
  tone?: "neutral" | "allowed" | "never";
  disabled?: boolean;
  /** Stable access-mode selector hook for native and Debug API regression drivers. */
  dataMode?: string;
  /** Stable option hook for real-input setup and regression drivers. */
  testId?: string;
}

/** Shared selector; the selected value remains controlled by its caller. */
export function Selector<T extends string>({
  label,
  value,
  options,
  onChange,
  onOpen,
  onClose,
  emptyLabel = "No choices available.",
  disabled = false,
  title,
  compact = false,
  includeValueInName = false,
  dataChip,
  ariaDescription,
  testId,
}: {
  label: string;
  value: T;
  options: readonly SelectorOption<T>[];
  /** The trust bit comes from the option's own native click, never the trigger. */
  onChange: (value: T, trustedClick: boolean) => void;
  onOpen?: () => void;
  onClose?: () => void;
  emptyLabel?: string;
  disabled?: boolean;
  title?: string;
  compact?: boolean;
  /** Include the visible current value in the trigger's name; keep the listbox label stable. */
  includeValueInName?: boolean;
  dataChip?: string;
  /** Optional selected-state wording in addition to the visible value description. */
  ariaDescription?: string;
  testId?: string;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const previousOpen = useRef(false);
  const [activeValue, setActiveValue] = useState<T | null>(null);
  const [position, setPosition] = useState({
    top: 0,
    left: 0,
    minWidth: 0,
    width: undefined as number | undefined,
    maxHeight: 280,
  });
  const selectedIndex = options.findIndex((option) => option.value === value);
  const selected = options[selectedIndex];
  const optionKeys = JSON.stringify(options.map((option) => [option.value, option.disabled]));
  const enabled = options.flatMap((option, i) => (option.disabled ? [] : [i]));
  const activeIndex = options.findIndex(
    (option) => option.value === activeValue && !option.disabled,
  );
  const index = activeIndex < 0 ? (enabled[0] ?? -1) : activeIndex;
  const close = (focus = true) => {
    setOpen(false);
    onClose?.();
    if (focus) trigger.current?.focus();
  };
  // Disabled is a render constraint. Adjust the local state before rendering children so an
  // enabled trigger cannot resurrect a menu that was hidden during a busy/blocked transition.
  if (disabled && open) setOpen(false);
  useEffect(() => {
    if (disabled && previousOpen.current) onClose?.();
    previousOpen.current = open;
  }, [disabled, open, onClose]);
  useEffect(() => {
    if (!open || disabled) return;
    const dialog = trigger.current?.closest('[role="dialog"][aria-modal="true"]');
    if (!dialog) return;
    const owned = dialog.getAttribute("aria-owns")?.split(/\s+/).filter(Boolean) ?? [];
    dialog.setAttribute("aria-owns", [...new Set([...owned, id])].join(" "));
    return () => {
      const remaining =
        dialog
          .getAttribute("aria-owns")
          ?.split(/\s+/)
          .filter((value) => value && value !== id) ?? [];
      if (remaining.length > 0) dialog.setAttribute("aria-owns", remaining.join(" "));
      else dialog.removeAttribute("aria-owns");
    };
  }, [disabled, id, open]);
  useLayoutEffect(() => {
    if (!open) return;
    const measure = () => {
      const bounds = trigger.current?.getBoundingClientRect();
      if (!bounds) return;
      const inset = 8;
      const footerTop = document.querySelector(".statusbar")?.getBoundingClientRect().top ?? 0;
      const bottom = footerTop > 0 ? Math.min(window.innerHeight, footerTop) : window.innerHeight;
      const height = Math.min(menu.current?.scrollHeight ?? 280, 280, bottom - 2 * inset);
      const width = Math.min(
        Math.max(bounds.width, menu.current?.offsetWidth ?? 0),
        window.innerWidth - 2 * inset,
      );
      const below = bottom - bounds.bottom - inset;
      const above = bounds.top - inset;
      const up = below < height && above > below;
      const maxHeight = Math.max(40, Math.min(280, up ? above - 4 : below - 4));
      setPosition({
        top: up ? Math.max(inset, bounds.top - Math.min(height, maxHeight) - 4) : bounds.bottom + 4,
        left: Math.max(inset, Math.min(bounds.left, window.innerWidth - width - inset)),
        minWidth: Math.min(bounds.width, window.innerWidth - 2 * inset),
        width,
        maxHeight,
      });
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [open, options.length]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !trigger.current?.contains(event.target) &&
        !menu.current?.contains(event.target)
      ) {
        setOpen(false);
        onClose?.();
      }
    };
    document.addEventListener("pointerdown", outside);
    return () => {
      document.removeEventListener("pointerdown", outside);
    };
  }, [open, onClose]);
  useEffect(() => {
    if (open) {
      const option = menu.current?.querySelector<HTMLButtonElement>(
        `[data-index="${String(index)}"]`,
      );
      (option ?? menu.current)?.focus();
    }
  }, [open, index, optionKeys]);
  const show = () => {
    if (disabled) return;
    onOpen?.();
    setActiveValue(
      (selected?.disabled ? undefined : selected?.value) ??
        options[enabled[0] ?? -1]?.value ??
        null,
    );
    setOpen(true);
  };
  return (
    <span className="ui-selector" data-compact={compact}>
      <button
        ref={trigger}
        id={`${id}-trigger`}
        type="button"
        value={value}
        className="ui-selector-trigger"
        role="combobox"
        data-chip={dataChip}
        data-testid={testId}
        aria-label={includeValueInName ? `${label}: ${selected?.label ?? value}` : label}
        aria-description={ariaDescription}
        aria-describedby={`${id}-value${title === undefined ? "" : ` ${id}-hint`}`}
        aria-haspopup="listbox"
        aria-expanded={open && !disabled}
        aria-controls={open && !disabled ? id : undefined}
        disabled={disabled}
        title={title ?? label}
        onClick={() => {
          if (open) close();
          else show();
        }}
        onKeyDown={(event) => {
          if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
            event.preventDefault();
            show();
            if (event.key === "Home") setActiveValue(options[enabled[0] ?? -1]?.value ?? null);
            if (event.key === "End")
              setActiveValue(options[enabled[enabled.length - 1] ?? -1]?.value ?? null);
          }
        }}
      >
        {selected?.icon && <Icon name={selected.icon} size={14} />}
        {selected?.tone && (
          <span className="ui-selector-dot" data-tone={selected.tone} aria-hidden="true" />
        )}
        <span id={`${id}-value`} className="ui-selector-text">
          {selected?.label ?? value}
        </span>
        <Icon name="chevron-down" size={12} />
      </button>
      {title !== undefined && (
        <span id={`${id}-hint`} className="sr">
          {title}
        </span>
      )}
      {open &&
        !disabled &&
        createPortal(
          <div
            ref={menu}
            id={id}
            data-selector-trigger={`${id}-trigger`}
            role="listbox"
            tabIndex={-1}
            aria-label={label}
            className="ui-selector-menu"
            style={position}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                close();
              } else if (event.key === "Tab") close();
              else if (
                enabled.length > 0 &&
                ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)
              ) {
                event.preventDefault();
                const next =
                  event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? enabled.length - 1
                      : (enabled.indexOf(index) +
                          (event.key === "ArrowDown" ? 1 : enabled.length - 1)) %
                        enabled.length;
                setActiveValue(options[enabled[next] ?? -1]?.value ?? null);
              }
            }}
          >
            {options.length === 0 && (
              <p className="ui-selector-empty" role="status">
                {emptyLabel}
              </p>
            )}
            {options.map((option, i) => (
              <button
                key={option.value}
                type="button"
                value={option.value}
                role="option"
                title={option.tooltip}
                aria-selected={option.value === value}
                disabled={option.disabled}
                tabIndex={i === index ? 0 : -1}
                data-index={i}
                data-mode={option.dataMode}
                data-testid={option.testId}
                className="ui-selector-option"
                onClick={(event) => {
                  onChange(option.value, event.nativeEvent.isTrusted);
                  close();
                }}
              >
                {option.icon && <Icon name={option.icon} size={14} />}
                {option.tone && (
                  <span className="ui-selector-dot" data-tone={option.tone} aria-hidden="true" />
                )}
                <span>
                  <span className="ui-selector-text">{option.label}</span>
                  {option.description && (
                    <span className="ui-selector-description">{option.description}</span>
                  )}
                </span>
                {option.value === value && <Icon name="check" size={14} />}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </span>
  );
}
