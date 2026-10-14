/**
 * Loads the composer view on first use: its views, menus and the `/` menu table live in their
 * own chunk, so the main bundle does not grow with them (the main chunk had reached 528 kB). The
 * send logic and the store stay in the main bundle (the `ui/composer` route and the palette need them
 * before any composer is shown).
 * Depends on: react (lazy, Suspense). Called by: the content area.
 * Invariant: the fallback reserves the composer's place and takes no input (nothing typed can be lost
 * while the chunk loads: it is a local file, a few milliseconds).
 */
import { type ReactElement, Suspense, lazy } from "react";

const Composer = lazy(() => import("./Composer"));

/** The composer of `sessionId`, loaded lazily. */
export function LazyComposer({ sessionId }: { sessionId: string }): ReactElement {
  return (
    <Suspense fallback={<div className="composer-wrap composer-loading" aria-hidden="true" />}>
      <Composer sessionId={sessionId} />
    </Suspense>
  );
}
