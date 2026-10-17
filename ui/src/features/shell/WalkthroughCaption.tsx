/** Debug-only walkthrough caption. Reads uiControl's store; Shell places it above the status bar. */
import { useStore } from "../../state/createStore";
import { walkthroughText } from "../../state/walkthrough";
import { useApp } from "./AppContext";

/** The non-interactive caption, absent during normal use. */
export function WalkthroughCaption() {
  const { uiControl } = useApp();
  const state = useStore(uiControl.walkthrough.store, (s) => s);
  if (state.phase === "off") return null;
  return (
    <div className="walkthrough-caption ui-panel ui-body" aria-hidden="true">
      {walkthroughText(state)}
    </div>
  );
}
