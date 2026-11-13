/**
 * The theme switch's cross-fade per platform.
 * PREDICT: on Linux (WebKitGTK) a view transition ends the window's web process, so the window freezes. Without a
 * platform check `switchTheme` starts one there too, and the Linux case below fails (a transition is started and the
 * theme is not applied at once). Windows and macOS keep the cross-fade.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { switchTheme } from "../../../ui/src/state/themeSwitch";

const doc = document as Document & {
  startViewTransition?: (cb: () => void) => unknown;
};

function withPlatform(platform: string, userAgent: string): void {
  vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
}

function withViewTransitions(): ReturnType<typeof vi.fn> {
  const start = vi.fn((callback: () => void) => {
    callback();
    return {};
  });
  Object.defineProperty(document, "startViewTransition", {
    configurable: true,
    value: start,
  });
  return start;
}

afterEach(() => {
  vi.restoreAllMocks();
  Reflect.deleteProperty(doc, "startViewTransition");
  delete document.documentElement.dataset["theme"];
});

describe("switchTheme", () => {
  it("applies the theme at once on Linux, without a view transition", () => {
    withPlatform(
      "Linux x86_64",
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)",
    );
    const start = withViewTransitions();
    const set = vi.fn();
    switchTheme("dark", set);
    expect(start).not.toHaveBeenCalled();
    expect(set).toHaveBeenCalledExactlyOnceWith("dark");
  });

  it("cross-fades on Windows and macOS", () => {
    for (const [platform, userAgent] of [
      [
        "Win32",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/140.0",
      ],
      [
        "MacIntel",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
      ],
    ] as const) {
      withPlatform(platform, userAgent);
      const start = withViewTransitions();
      const set = vi.fn();
      switchTheme("light", set);
      expect(start, platform).toHaveBeenCalledOnce();
      expect(set, platform).toHaveBeenCalledExactlyOnceWith("light");
      expect(document.documentElement.dataset["theme"], platform).toBe("light");
      vi.restoreAllMocks();
    }
  });
});
