/**
 * Last-resort window recovery for React render and lifecycle errors. Renders with React and the
 * shared Button; main mounts it outside the app provider. Optional diagnostic wiring is isolated.
 * Retrying remounts the shell with its existing stores; reloading reconnects the whole window.
 */
import { Component, type ReactNode, useEffect } from "react";

import { Button } from "../../components/Controls";
import type { App } from "./AppContext";
import { startRecoveryWiring } from "./recoveryWiring";
import { FeedbackAction } from "../feedback/FeedbackAction";
import { CoreFeedbackDialog } from "../feedback/FeedbackDialog";

function Recovery({
  app,
  retry,
  error,
}: {
  app: App | undefined;
  retry: () => void;
  error: string;
}) {
  useEffect(() => (app === undefined ? undefined : startRecoveryWiring(app)), [app]);
  return (
    <main className="setup" role="alert">
      <div className="setup-step">
        <h2>The window could not be displayed</h2>
        <Button
          data-testid="window-error.retry"
          shape="outlined"
          variant="primary"
          autoFocus
          onClick={retry}
        >
          Try again
        </Button>
        <Button
          data-testid="window-error.reload"
          shape="outlined"
          onClick={() => {
            window.location.reload();
          }}
        >
          Reload window
        </Button>
        <FeedbackAction core={app?.core} title="The window could not be displayed" text={error} />
      </div>
      <CoreFeedbackDialog core={app?.core} />
    </main>
  );
}

/** Keeps a failed render from leaving the window blank. */
export class AppErrorBoundary extends Component<
  { children: ReactNode; app?: App },
  { failed: boolean; error: string }
> {
  override state = { failed: false, error: "" };

  static getDerivedStateFromError(error: unknown): { failed: boolean; error: string } {
    return {
      failed: true,
      error: error instanceof Error ? error.message : "The window could not be displayed.",
    };
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return (
      <Recovery
        app={this.props.app}
        error={this.state.error}
        retry={() => {
          this.setState({ failed: false });
        }}
      />
    );
  }
}
