/** Shared tab-list, tab and close controls; features retain their existing selection, drag and keyboard behavior. Uses native semantics and presentation classes. */
import { createContext, useContext, type ComponentProps } from "react";
const PRESENTATIONS = {
  segmented: ["ui-segments", "ui-segment", ""],
  underline: ["ttabs", "ttab", ""],
  pill: ["ptabs", "ptab", "ptab-x"],
  strip: ["vw-term-tabs", "vw-term-tab", ""],
  pages: ["browser-page-tabs", "browser-page-tab-select", "browser-page-tab-close"],
} as const;
type Variant = keyof typeof PRESENTATIONS;
const Presentation = createContext<Variant>("underline");
type TabProps = Omit<ComponentProps<"button">, "className" | "role"> & { "data-testid": string };

/** A tab strip with no additional DOM around the existing row. */
export function Tabs({
  variant = "underline",
  children,
  onKeyDown,
  ...props
}: Omit<ComponentProps<"div">, "className" | "role"> & {
  variant?: Variant;
  "data-testid"?: string;
}) {
  return (
    <Presentation value={variant}>
      <div
        {...props}
        data-testid={props["data-testid"]}
        className={PRESENTATIONS[variant][0]}
        role="tablist"
        onKeyDown={(event) => {
          onKeyDown?.(event);
          if (variant !== "segmented" || event.defaultPrevented) return;
          const tabs = [
            ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
              '[role="tab"]:not(:disabled)',
            ),
          ];
          const index = tabs.indexOf(event.target as HTMLButtonElement);
          if (index < 0) return;
          const next =
            event.key === "Home"
              ? 0
              : event.key === "End"
                ? tabs.length - 1
                : event.key === "ArrowRight"
                  ? (index + 1) % tabs.length
                  : event.key === "ArrowLeft"
                    ? (index + tabs.length - 1) % tabs.length
                    : null;
          if (next === null) return;
          event.preventDefault();
          tabs[next]?.focus();
          tabs[next]?.click();
        }}
      >
        {children}
      </div>
    </Presentation>
  );
}
/** A named native tab; aria-selected is the feature's confirmed state. */
export function Tab(props: TabProps) {
  const variant = useContext(Presentation);
  return (
    <button
      type="button"
      {...props}
      tabIndex={
        variant === "segmented"
          ? props["aria-selected"] === true || props["aria-selected"] === "true"
            ? 0
            : -1
          : props.tabIndex
      }
      data-testid={props["data-testid"]}
      className={`ui-tab ${PRESENTATIONS[variant][1]}`}
      role="tab"
    />
  );
}
/** A close control beside a tab, retaining its own accessible name. */
export function TabClose(props: TabProps) {
  const variant = useContext(Presentation);
  return (
    <button
      type="button"
      {...props}
      data-testid={props["data-testid"]}
      className={`ui-tab-close ${PRESENTATIONS[variant][2]}`}
    />
  );
}
