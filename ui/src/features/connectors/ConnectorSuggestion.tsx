/** Shows a completed agent suggestion beside its steps; Review selects the connector's existing detail page. */
import { Button } from "../../components/Controls";
import { connectorSuggestionOf } from "../../state/connectorSuggestions";
import type { Obj } from "../transcript/model/entries";
import { useApp } from "../shell/AppContext";

/** Keeps the offer visible while the raw tool steps are collapsed. */
export function ConnectorSuggestion({ entry }: { entry: Obj }) {
  const { connectors } = useApp();
  const suggestion = connectorSuggestionOf(entry);
  if (suggestion === null || connectors === null) return null;
  return (
    <div className="server-note">
      <span>
        <b>Use {suggestion.name}?</b> {suggestion.reason}
      </span>
      <Button
        data-testid="connectors.suggestion-review"
        data-item-id={suggestion.alias}
        type="button"
        onClick={() => {
          void connectors.actions.review(suggestion.alias);
        }}
      >
        Review {suggestion.name}
      </Button>
    </div>
  );
}
