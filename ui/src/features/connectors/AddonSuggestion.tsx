/** Shows the suggested browser's live setup control; the person decides whether to connect it. */
import { addonSuggestionOf } from "../../state/addonSuggestions";
import { TEXT } from "../../vocabulary";
import type { Obj } from "../transcript/model/entries";
import { ChromeConnection } from "../settings/chrome/ChromeConnection";
import { chromeAvailable } from "../settings/chrome/chromeRows";
import { useChromeControl } from "../settings/chrome/useChromeControl";

/** The live control (its state comes from the core, not from the suggestion), with the agent's reason. */
function BrowserSuggestion({ addon, reason }: { addon: "chrome" | "edge"; reason: string }) {
  const edge = addon === "edge";
  const chrome = useChromeControl(edge ? "Edge" : "Chrome");
  if (!chromeAvailable(chrome.view)) return null;
  return (
    <div className="server-note" data-testid="addon-suggestion" data-item-id={addon}>
      <span>
        <b>{edge ? TEXT.edgeName : TEXT.chromeName}</b> · {reason}
      </span>
      <ChromeConnection chrome={chrome} browser={edge ? "Edge" : "Chrome"} />
    </div>
  );
}

/** Keeps the offer visible while the raw tool steps are collapsed. */
export function AddonSuggestion({ entry }: { entry: Obj }) {
  const suggestion = addonSuggestionOf(entry);
  if (suggestion === null) return null;
  return <BrowserSuggestion addon={suggestion.addon} reason={suggestion.reason} />;
}
