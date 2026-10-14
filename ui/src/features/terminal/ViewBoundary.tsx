/**
 * Catches a failure while mounting or rendering a terminal view, so the side panel shows what went
 * wrong instead of an empty area (React error boundaries are still class components).
 * Depends on: react. Called by: TerminalPanel (one per shown terminal, keyed by its id).
 * Invariant: only the view is replaced; the terminal keeps running in the core.
 */
import { Component, type ReactNode } from "react";

interface State {
  message: string | null;
}

/** Shows `children`, or the error message once they throw. */
export class ViewBoundary extends Component<{ children: ReactNode }, State> {
  override state: State = { message: null };

  static getDerivedStateFromError(error: unknown): State {
    return { message: error instanceof Error ? error.message : String(error) };
  }

  override render(): ReactNode {
    if (this.state.message === null) return this.props.children;
    return (
      <div className="vw-term-error" role="alert">
        {`The terminal view failed: ${this.state.message}`}
      </div>
    );
  }
}
