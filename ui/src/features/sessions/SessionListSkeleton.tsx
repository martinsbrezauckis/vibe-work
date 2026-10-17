/** Tree-shaped loading rows and the delayed note; uses shared motion styles and the core's readiness. Called by SessionList. */
import { useEffect, useState } from "react";

/** Mounted only while the list is loading, so each load gets its own six-second delay. */
export function SessionListSkeleton({ ready }: { ready: boolean }) {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => {
      setSlow(true);
    }, 6000);
    return () => {
      clearTimeout(timer);
    };
  }, []);
  return (
    <>
      <div role="status">
        <span className="sr">Loading your sessions…</span>
        <div aria-hidden="true">
          {[
            { header: "55%", rows: ["70%", "50%", "62%", "44%"] },
            { header: "44%", rows: ["62%", "50%"] },
          ].map((group) => (
            <div key={group.header}>
              <div className="sec">
                <span className="sk" style={{ width: group.header }} />
              </div>
              {group.rows.map((width) => (
                <div className="sk-row" key={width}>
                  <span className="sk" style={{ width }} />
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
      {slow && (
        <p className="col-note">
          {ready ? "Still loading sessions from Vibe." : "Still loading. Vibe is starting."}
        </p>
      )}
    </>
  );
}
