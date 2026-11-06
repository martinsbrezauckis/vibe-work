/**
 * Last-resort window recovery for React render and lifecycle errors. Renders with React and the
 * shared Button; main mounts it outside the app provider. Optional diagnostic wiring is isolated.
 * Retrying remounts the shell with its existing stores; reloading reconnects the whole window.
 */
import { Component, type ReactNode, useEffect } from "react";

import { Button } from "../../components/Controls";
import type { App } from "./AppContext";
import { startRecoveryWiring } from "./recoveryWiring";

function Recovery({ app, retry }: { app: App | undefined; retry: () => void }) {
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
      </div>
    </main>
  );
}

/** Keeps a failed render from leaving the window blank. */
export class AppErrorBoundary extends Component<
  { children: ReactNode; app?: App },
  { failed: boolean }
> {
  override state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return (
      <Recovery
        app={this.props.app}
        retry={() => {
          this.setState({ failed: false });
        }}
      />
    );
  }
}
