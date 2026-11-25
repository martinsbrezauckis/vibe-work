/**
 * On-demand chunk wrapper for non-first-screen pages and panel tabs: loads the component on
 * first mount (or ahead of time via `preload()`), hides fast loads (<150 ms) so local chunks never
 * flash, keeps input outside unblocked while loading, and shows a readable reason with Retry if
 * the chunk fails to load or renders with an error.
 * Depends on: react, components/Controls (`Button`). Called by: Content, SessionList, panel tabs, VoiceChatTag.
 */
import {
  Component,
  type ComponentType,
  type ReactElement,
  type ReactNode,
  useEffect,
  useReducer,
  useState,
} from "react";

import { CHUNK_HELP, chunkFailureLine, chunkFailureReason } from "./chunkError";
import { Button } from "./Controls";

const FLASH_GUARD_MS = 150;

/** Extracts a single-sentence readable failure reason without crashing on non-Error throws. */
function chunkErrorText(error: unknown): string {
  const reason = chunkFailureReason(error);
  return reason === "" ? CHUNK_HELP : reason;
}

function LazyErrorBox({
  label,
  testIdPrefix,
  error,
  onRetry,
}: {
  label: string;
  testIdPrefix: string;
  error: string;
  onRetry: () => void;
}): ReactElement {
  return (
    <div className="lazy-error" role="alert" data-testid={`${testIdPrefix}.error`}>
      <p className="lazy-error-text">{chunkFailureLine(label, error)}</p>
      <Button data-testid={`${testIdPrefix}.retry`} shape="outlined" onClick={onRetry}>
        Try again
      </Button>
    </div>
  );
}

class ChunkBoundary extends Component<
  {
    resetKey: number;
    label: string;
    testIdPrefix: string;
    onRetry: () => void;
    children: ReactNode;
  },
  { error: string | null }
> {
  override state: { error: string | null } = { error: null };

  static getDerivedStateFromError(error: unknown): { error: string } {
    return { error: chunkErrorText(error) };
  }

  override componentDidUpdate(prevProps: { resetKey: number }): void {
    if (prevProps.resetKey !== this.props.resetKey && this.state.error !== null) {
      this.setState({ error: null });
    }
  }

  override render(): ReactNode {
    if (this.state.error === null) return this.props.children;
    return (
      <LazyErrorBox
        label={this.props.label}
        testIdPrefix={this.props.testIdPrefix}
        error={this.state.error}
        onRetry={() => {
          this.setState({ error: null });
          this.props.onRetry();
        }}
      />
    );
  }
}

/** Lazily loaded component wrapper exposing test preload and reset hooks. */
export interface LazyComponent<P extends object> {
  (props: P): ReactElement | null;
  /** Preloads the chunk so subsequent renders commit synchronously without fallback UI. */
  preload: () => Promise<void>;
  /** Clears cached module/error state (for deterministic lazy/retry unit tests). */
  resetForTest: () => void;
}

/**
 * Wraps a dynamic component import with synchronous cache hits, a delayed non-blocking loading
 * indicator, and readable error recovery with Retry.
 */
export function createLazyComponent<P extends object>(options: {
  label: string;
  testIdPrefix: string;
  load: () => Promise<{ default: ComponentType<P> }>;
  delayMs?: number;
}): LazyComponent<P> {
  const { label, testIdPrefix, load, delayMs = FLASH_GUARD_MS } = options;
  let loaded: ComponentType<P> | null = null;
  let inFlight: Promise<ComponentType<P>> | null = null;
  let lastError: string | null = null;

  const ensureLoaded = (): Promise<ComponentType<P>> => {
    if (loaded !== null) return Promise.resolve(loaded);
    if (inFlight !== null) return inFlight;
    lastError = null;
    inFlight = load()
      .then((mod) => {
        loaded = mod.default;
        lastError = null;
        inFlight = null;
        return mod.default;
      })
      .catch((error: unknown) => {
        inFlight = null;
        lastError = chunkErrorText(error);
        throw error;
      });
    return inFlight;
  };

  function LazyView(props: P): ReactElement | null {
    const [, bump] = useReducer((n: number) => n + 1, 0);
    const [attempt, setAttempt] = useState(0);
    const [showLoading, setShowLoading] = useState(
      () => loaded === null && lastError === null && delayMs <= 0,
    );

    useEffect(() => {
      if (loaded !== null) return;
      let alive = true;
      const timer =
        delayMs <= 0
          ? null
          : setTimeout(() => {
              if (alive && loaded === null && lastError === null) setShowLoading(true);
            }, delayMs);

      ensureLoaded()
        .then(() => {
          if (!alive) return;
          if (timer !== null) clearTimeout(timer);
          setShowLoading(false);
          bump();
        })
        .catch(() => {
          if (!alive) return;
          if (timer !== null) clearTimeout(timer);
          setShowLoading(false);
          bump();
        });

      return () => {
        alive = false;
        if (timer !== null) clearTimeout(timer);
      };
    }, [attempt]);

    const retry = () => {
      loaded = null;
      lastError = null;
      inFlight = null;
      setShowLoading(delayMs <= 0);
      setAttempt((n) => n + 1);
    };

    if (loaded !== null) {
      const Resolved = loaded;
      return (
        <ChunkBoundary resetKey={attempt} label={label} testIdPrefix={testIdPrefix} onRetry={retry}>
          <Resolved {...props} />
        </ChunkBoundary>
      );
    }

    if (lastError !== null) {
      return (
        <LazyErrorBox label={label} testIdPrefix={testIdPrefix} error={lastError} onRetry={retry} />
      );
    }

    if (!showLoading) return null;
    return (
      <div
        className="lazy-status"
        role="status"
        aria-live="polite"
        data-testid={`${testIdPrefix}.loading`}
      >
        Loading…
      </div>
    );
  }

  LazyView.preload = async (): Promise<void> => {
    await ensureLoaded();
  };

  LazyView.resetForTest = (): void => {
    loaded = null;
    inFlight = null;
    lastError = null;
  };

  return LazyView;
}
