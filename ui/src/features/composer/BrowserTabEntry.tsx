/** The tab picker entry point and browser-local readiness dots. State comes from core facts; no composer notice. */
import { Button } from "../../components/Controls";
import { TEXT } from "../../vocabulary";
import { useApp } from "../shell/AppContext";
import { useCoreState } from "../shell/hooks";
import { browserSignals, openBrowserTabs, usableBrowsers } from "./browserTabs";

export function BrowserTabEntry({ sessionId }: { sessionId: string }) {
  const app = useApp();
  const view = useCoreState((state) => state.chromeControl);
  const signals = browserSignals(view);
  const usable = usableBrowsers(view).length > 0;
  if (signals.length === 0 && !usable) return null;
  const description = [
    ...signals.map((signal) => `${signal.browser}: ${signal.label}`),
    ...(view?.selectionError ? ["Selection unavailable"] : []),
  ].join(" · ");
  return (
    <Button
      type="button"
      appearance="choice"
      data-testid="composer.browser-tabs-open"
      aria-label={TEXT.addBrowserTab}
      aria-description={description}
      title={description}
      onClick={() => {
        if (usable) void openBrowserTabs(app, sessionId, "window");
        else app.shell.actions.openSettings("browser");
      }}
    >
      {TEXT.addBrowserTab}
      <span className="browser-signals">
        {signals.map(({ browser, state, label }) => (
          <span
            key={browser}
            className="browser-signal"
            role="img"
            aria-label={`${browser}: ${label}`}
            title={`${browser}: ${label}`}
            data-state={state}
            data-testid="composer.browser-signal"
            data-item-id={browser}
          />
        ))}
      </span>
    </Button>
  );
}
