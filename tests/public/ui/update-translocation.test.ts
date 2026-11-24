/** Translocated Mac copies explain the move and never offer an impossible update action.
 * Depends on the pure update notice model; run by the public UI suite.
 */
import { describe, expect, it } from "vitest";
import type { UpdateView } from "../../../ui/src/protocol/client/types";
import { noticeFor, statusLine } from "../../../ui/src/state/updates";

const headline = "Move Vibe Work to Applications to update.";
const detail =
  "This copy is running from a temporary read-only place. Move Vibe Work into Applications, then open it from there to update.";
const releasePage =
  "https://github.com/martinsbrezauckis/vibe-work/releases/tag/v0.1.17";

function view(
  phase: UpdateView["phase"],
  link: string | null = releasePage,
): UpdateView {
  return {
    current: "0.1.16",
    // The host reports this mode for a translocated Mac app.
    mode: "macosTranslocated",
    autoDownload: true,
    releasePage: link,
    lastChecked: 1,
    phase,
  };
}

describe("a Mac copy running from a temporary read-only place", () => {
  it.each<UpdateView["phase"]>([
    { type: "available", version: "0.1.17", notes: "New features." },
    { type: "ready", version: "0.1.17", notes: null },
    {
      type: "failed",
      stage: "install",
      version: "0.1.17",
      error:
        'Read-only file system (os error 30) at path "/private/var/folders/AppTranslocation/tauri_current_app"',
    },
  ])(
    "guides instead of offering download, install or retry: $type",
    (phase) => {
      expect(noticeFor(view(phase))).toMatchObject({
        tone: "info",
        headline,
        detail,
        progress: null,
        actions: ["openReleasePage", "later"],
      });
      expect(statusLine(view(phase))).toBe(`${headline} ${detail}`);
    },
  );

  it("still explains the move when the release link is missing", () => {
    expect(
      noticeFor(view({ type: "ready", version: "0.1.17", notes: null }, null)),
    ).toMatchObject({ headline, detail, actions: ["later"] });
  });

  it.each<UpdateView["phase"]>([
    { type: "idle" },
    { type: "checking" },
    { type: "failed", stage: "check", error: "Offline", version: null },
  ])("does not invent an update offer for $type", (phase) => {
    expect(noticeFor(view(phase))).toBeNull();
  });

  it.each(["inApp", "linkOnly"] as const)(
    "keeps the existing %s offer",
    (mode) => {
      const normal = {
        ...view({ type: "available", version: "0.1.17", notes: null }),
        mode,
      };
      expect(noticeFor(normal)?.actions).toEqual(
        mode === "inApp" ? ["download", "later"] : ["openReleasePage", "later"],
      );
    },
  );
});
