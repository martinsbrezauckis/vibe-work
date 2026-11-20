// PREDICT: the built-in recorder must check VP8 support before accepting frames and must reject an unavailable codec.
import { afterEach, beforeAll, expect, test, vi } from "vitest";

import { supportedVideoConfig } from "../../../ui/src/features/preview/browserVideo";
import {
  VIDEO_LIMITS,
  WebmRecorder,
} from "../../../ui/src/features/preview/video-encoder";
import { installAppStyles } from "./support/appStyles";

beforeAll(installAppStyles);

afterEach(() => {
  vi.unstubAllGlobals();
  document.documentElement.removeAttribute("data-theme");
});

// PREDICT: the encoder's frame ceiling differs from the shared 1,200-frame contract.
test("the encoder enforces the shared video frame ceiling", () => {
  expect(VIDEO_LIMITS.frames).toBe(1200);
});

test("VP8 availability is checked by WebCodecs at recording start", async () => {
  const supported = vi.fn(() => Promise.resolve({ supported: true }));
  vi.stubGlobal("VideoEncoder", { isConfigSupported: supported });
  vi.stubGlobal(
    "OffscreenCanvas",
    class {
      getContext() {
        return {};
      }
    },
  );
  vi.stubGlobal("createImageBitmap", () => undefined);
  vi.stubGlobal("VideoFrame", () => undefined);
  expect(await supportedVideoConfig()).toBe(true);
  expect(supported).toHaveBeenCalledWith(
    expect.objectContaining({ codec: "vp8", width: 640, height: 480 }),
  );
});

test("missing or unsupported WebCodecs leaves the action GIF available", async () => {
  vi.stubGlobal(
    "OffscreenCanvas",
    class {
      getContext() {
        return {};
      }
    },
  );
  vi.stubGlobal("createImageBitmap", () => undefined);
  vi.stubGlobal("VideoFrame", () => undefined);
  vi.stubGlobal("VideoEncoder", undefined);
  expect(await supportedVideoConfig()).toBe(false);
  vi.stubGlobal("VideoEncoder", {
    isConfigSupported: () => Promise.resolve({ supported: false }),
  });
  expect(await supportedVideoConfig()).toBe(false);
});

// PREDICT: the first-frame reply must fail if bitmap decoding fails, so Rust cancels before reporting a started video.
test("a failed first frame is reported at start", async () => {
  vi.stubGlobal("createImageBitmap", () =>
    Promise.reject(new Error("decode failed")),
  );
  const recorder = new WebmRecorder();
  await expect(recorder.add("invalid", 0)).rejects.toThrow("decode failed");
});

// PREDICT: a failed encoder flush must close the encoder before UI drops the recording.
test("a rejected flush closes the video encoder", async () => {
  const close = vi.fn();
  vi.stubGlobal("createImageBitmap", () =>
    Promise.resolve({ width: 8, height: 8, close: vi.fn() }),
  );
  vi.stubGlobal(
    "OffscreenCanvas",
    class {
      width: number;
      height: number;
      constructor(width: number, height: number) {
        this.width = width;
        this.height = height;
      }
      getContext() {
        return { fillRect: vi.fn(), drawImage: vi.fn() };
      }
    },
  );
  vi.stubGlobal(
    "VideoFrame",
    class {
      close = vi.fn();
    },
  );
  vi.stubGlobal(
    "VideoEncoder",
    class {
      state = "configured";
      encodeQueueSize = 0;
      configure = vi.fn();
      encode = vi.fn();
      flush = vi.fn(() => Promise.reject(new Error("flush failed")));
      close = close;
    },
  );
  const recorder = new WebmRecorder();
  await recorder.add("AA==", 0);
  await expect(recorder.finish(1)).rejects.toThrow("flush failed");
  expect(close).toHaveBeenCalledOnce();
});

// PREDICT: stopping long after frame capture must keep timestamps and duration within five minutes,
// and the final hold frame cannot raise the encoded frame count beyond the ceiling.
test("a late video stop and hold frame stay within the duration and frame bounds", async () => {
  const encodedAt: number[] = [];
  const muxedDuration = (webm: Uint8Array): number => {
    const offset = webm.findIndex(
      (value, index) =>
        value === 0x44 && webm[index + 1] === 0x89 && webm[index + 2] === 0x88,
    );
    expect(offset).toBeGreaterThanOrEqual(0);
    return new DataView(
      webm.buffer,
      webm.byteOffset + offset + 3,
      8,
    ).getFloat64(0);
  };
  vi.stubGlobal("createImageBitmap", () =>
    Promise.resolve({ width: 8, height: 8, close: vi.fn() }),
  );
  vi.stubGlobal(
    "OffscreenCanvas",
    class {
      width: number;
      height: number;
      constructor(width: number, height: number) {
        this.width = width;
        this.height = height;
      }
      getContext() {
        return { fillRect: vi.fn(), drawImage: vi.fn() };
      }
    },
  );
  vi.stubGlobal(
    "VideoFrame",
    class {
      timestamp: number;
      close = vi.fn();
      constructor(_canvas: unknown, init: { timestamp: number }) {
        this.timestamp = init.timestamp;
      }
    },
  );
  vi.stubGlobal(
    "VideoEncoder",
    class {
      state = "configured";
      encodeQueueSize = 0;
      constructor(private readonly init: VideoEncoderInit) {}
      configure = vi.fn();
      encode = vi.fn((frame: VideoFrame) => {
        encodedAt.push(frame.timestamp);
        this.init.output({
          byteLength: 4,
          timestamp: frame.timestamp,
          type: "key",
          copyTo: (dest: Uint8Array) => {
            dest.set([1, 2, 3, 4]);
          },
        } as EncodedVideoChunk);
      });
      flush = vi.fn(() => Promise.resolve());
      close = vi.fn();
    },
  );
  const recorder = new WebmRecorder();
  await recorder.add("AA==", 0);
  await recorder.add("AA==", 3600);
  const result = await recorder.finish(3600);
  expect(Math.max(...encodedAt)).toBeLessThanOrEqual((300_000 - 33) * 1000);
  expect(result.summary.durationMs).toBeLessThanOrEqual(300_000);
  const beforeCap = encodedAt.length;
  const capped = new WebmRecorder();
  for (let frame = 0; frame < VIDEO_LIMITS.frames; frame++) {
    await capped.add("AA==", frame * 0.25);
  }
  const atCap = await capped.finish(300);
  expect(atCap.summary.frames).toBe(VIDEO_LIMITS.frames);
  expect(encodedAt.length - beforeCap).toBe(VIDEO_LIMITS.frames);
  expect(muxedDuration(atCap.bytes)).toBeLessThanOrEqual(300_000);
  // A final frame close to Stop leaves less than HOLD_MS; the explicit Stop time is authoritative.
  const nearStop = new WebmRecorder();
  await nearStop.add("AA==", 0);
  await nearStop.add("AA==", 1.99);
  const ended = await nearStop.finish(2);
  expect(ended.summary.durationMs).toBeLessThanOrEqual(2000);
  expect(muxedDuration(ended.bytes)).toBe(ended.summary.durationMs);
});

// PREDICT: real app styles define one fixed white recording matte for both themes, and frame encoding reads it.
test("recording frames use the fixed media matte in light and dark themes", async () => {
  const fills: string[] = [];
  vi.stubGlobal("createImageBitmap", () =>
    Promise.resolve({ width: 8, height: 8, close: vi.fn() }),
  );
  vi.stubGlobal(
    "OffscreenCanvas",
    class {
      width: number;
      height: number;
      constructor(width: number, height: number) {
        this.width = width;
        this.height = height;
      }
      getContext() {
        return {
          set fillStyle(value: string) {
            fills.push(value);
          },
          fillRect: vi.fn(),
          drawImage: vi.fn(),
        };
      }
    },
  );
  vi.stubGlobal(
    "VideoFrame",
    class {
      close = vi.fn();
    },
  );
  vi.stubGlobal(
    "VideoEncoder",
    class {
      state = "configured";
      encodeQueueSize = 0;
      configure = vi.fn();
      encode = vi.fn();
      close = vi.fn();
    },
  );
  for (const theme of ["dark", "light"]) {
    document.documentElement.dataset.theme = theme;
    const matte = getComputedStyle(document.documentElement)
      .getPropertyValue("--recording-matte")
      .trim();
    expect(matte).toBe("#fff");
    const recorder = new WebmRecorder();
    await recorder.add("AA==", 0);
    expect(fills.at(-1)).toBe(matte);
    recorder.close();
  }
});
