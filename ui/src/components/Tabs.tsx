/** Shared tab-list, tab and close controls; features retain their existing selection, drag and keyboard behavior. Uses native semantics and presentation classes. */
import { createContext, useContext, type ComponentProps } from "react";
const PRESENTATIONS = {
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
  ...props
}: Omit<ComponentProps<"div">, "className" | "role"> & { variant?: Variant }) {
  return (
    <Presentation value={variant}>
      <div {...props} className={PRESENTATIONS[variant][0]} role="tablist">
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
      data-testid={props["data-testid"]}
      className={PRESENTATIONS[variant][1]}
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
      className={PRESENTATIONS[variant][2]}
    />
  );
}
