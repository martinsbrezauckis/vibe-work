/** Shows the suggested browser's live setup control; the person decides whether to connect it. */
import { addonSuggestionOf } from "../../state/addonSuggestions";
import { TEXT } from "../../vocabulary";
import type { Obj } from "../transcript/model/entries";
import { Button, SettingsRow } from "../../components/Controls";
import { useApp } from "../shell/AppContext";
import { ChromeConnection } from "../settings/chrome/ChromeConnection";
import { chromeAvailable, setupLine } from "../settings/chrome/chromeRows";
import { useChromeControl } from "../settings/chrome/useChromeControl";

/** The live control (its state comes from the core, not from the suggestion), with the agent's reason. */
function BrowserSuggestion({ addon, reason }: { addon: "chrome" | "edge"; reason: string }) {
  const { shell } = useApp();
  const edge = addon === "edge";
  const chrome = useChromeControl(edge ? "Edge" : "Chrome");
  if (!chromeAvailable(chrome.view)) return null;
  return (
    <div className="server-note" data-testid="addon-suggestion" data-item-id={addon}>
      <span>
        <b>{edge ? TEXT.edgeName : TEXT.chromeName}</b> · {reason}
      </span>
      {edge &&
      chrome.view?.setup.browserIssue === undefined &&
      chrome.view?.setup.addon?.action !== "remove" ? (
        <SettingsRow>
          <span>
            Connection
            <span className="d" role="status">
              {setupLine(chrome.view, "Edge")}
            </span>
          </span>
          <Button
            data-testid="addon.edge-connect"
            shape="outlined"
            onClick={() => {
              shell.actions.openSettings("browser");
            }}
          >
            Connect Edge
          </Button>
        </SettingsRow>
      ) : (
        <ChromeConnection chrome={chrome} browser={edge ? "Edge" : "Chrome"} />
      )}
    </div>
  );
}

/** Keeps the offer visible while the raw tool steps are collapsed. */
export function AddonSuggestion({ entry }: { entry: Obj }) {
  const suggestion = addonSuggestionOf(entry);
  if (suggestion === null) return null;
  return <BrowserSuggestion addon={suggestion.addon} reason={suggestion.reason} />;
}
