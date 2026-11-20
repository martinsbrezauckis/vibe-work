// Adapted from chrome-control/extension/src/adapter/video-encoder.ts at int0115 for Vibe Work's trusted main webview.
// Guarded native PNG screenshots in, a WebM file out (webm-muxer.ts). The first
// frame fixes the video size; later frames of another size are scaled into it. Bounded by bytes and frames; when
// a bound is reached the recorder keeps what it has and reports the limit. No audio, no other source than the
// frames the Rust browser sends after its capture checks. Times are seconds since start; finish repeats the
// last frame near the end so normal players show the full recorded duration (as in helper int0115).
import { muxWebm, type EncodedChunk } from "./webm-muxer";

/** Bounds and encoder settings for one in-memory browser video recording. */
export const VIDEO_LIMITS = {
  bytes: 24 * 1024 * 1024,
  frames: 1200,
  durationMs: 300_000,
  keyFrameMs: 2000,
  maxSide: 1280,
  bitrate: 2_000_000,
} as const;
/** Frame counts, output size, and any limit reached by a finished recording. */
export type VideoSummary = {
  frames: number;
  dropped: number;
  durationMs: number;
  width: number;
  height: number;
  bytes: number;
  limit?: "size" | "frames";
};
const HOLD_MS = 33;

function decodeBase64(value: string): Uint8Array {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Encodes ordered guarded PNG screenshots into one VP8 WebM recording. */
export class WebmRecorder {
  private encoder?: VideoEncoder;
  private canvas?: OffscreenCanvas;
  private context?: OffscreenCanvasRenderingContext2D;
  private readonly chunks: EncodedChunk[] = [];
  private lastMs = -1;
  private lastKeyMs = -Infinity;
  private bytes = 0;
  private frames = 0;
  private dropped = 0;
  private failure?: Error;
  private queue: Promise<void> = Promise.resolve();
  limit?: "size" | "frames";

  /** Queue one guarded screenshot (base64 PNG, seconds since recording start); frames are encoded strictly in order. */
  add(pngBase64: string, timestampSeconds: number): Promise<void> {
    this.queue = this.queue
      .then(() => this.encode(pngBase64, timestampSeconds))
      .catch((error: unknown) => {
        this.failure ??= error instanceof Error ? error : new Error(String(error));
      });
    return this.queue.then(() => {
      if (this.failure) throw this.failure;
    });
  }

  private async encode(pngBase64: string, timestampSeconds: number): Promise<void> {
    if (this.failure || this.limit) return;
    if (this.frames >= VIDEO_LIMITS.frames) {
      this.limit = "frames";
      return;
    }
    const bitmap = await createImageBitmap(
      new Blob([decodeBase64(pngBase64) as BlobPart], { type: "image/png" }),
    );
    try {
      const ms = this.frames
        ? Math.min(Math.round(timestampSeconds * 1000), VIDEO_LIMITS.durationMs - HOLD_MS)
        : 0;
      // Screencast frames can repeat a timestamp; a video frame needs a later time than the previous one.
      if (ms <= this.lastMs) {
        this.dropped++;
        return;
      }
      if (!this.encoder) this.configure(bitmap.width, bitmap.height);
      const encoder = this.encoder,
        canvas = this.canvas,
        context = this.context;
      if (!encoder || !canvas || !context) throw new Error("Video encoder did not initialize");
      if (encoder.encodeQueueSize > 8) {
        this.dropped++;
        return;
      }
      const matte = getComputedStyle(document.documentElement)
        .getPropertyValue("--recording-matte")
        .trim();
      if (!matte) throw new Error("Recording matte is unavailable");
      context.fillStyle = matte;
      context.fillRect(0, 0, canvas.width, canvas.height);
      context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      this.encodeCanvas(ms);
      this.frames++;
    } finally {
      bitmap.close();
    }
  }

  private encodeCanvas(ms: number): void {
    const encoder = this.encoder,
      canvas = this.canvas;
    if (!encoder || !canvas) throw new Error("Video encoder did not initialize");
    const frame = new VideoFrame(canvas, { timestamp: ms * 1000 });
    const key = ms - this.lastKeyMs >= VIDEO_LIMITS.keyFrameMs;
    try {
      encoder.encode(frame, { keyFrame: key });
    } finally {
      frame.close();
    }
    if (key) this.lastKeyMs = ms;
    this.lastMs = ms;
  }

  private configure(width: number, height: number): void {
    const scale = Math.min(1, VIDEO_LIMITS.maxSide / Math.max(width, height));
    // VP8 frames use even dimensions.
    const w = Math.max(2, Math.floor((width * scale) / 2) * 2),
      h = Math.max(2, Math.floor((height * scale) / 2) * 2);
    this.canvas = new OffscreenCanvas(w, h);
    const context = this.canvas.getContext("2d");
    if (!context) throw new Error("Video canvas is unavailable");
    this.context = context;
    this.encoder = new VideoEncoder({
      output: (chunk) => {
        if (this.limit === "size") return;
        const data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        if (this.bytes + data.length > VIDEO_LIMITS.bytes) {
          this.limit = "size";
          return;
        }
        // Muxing needs a key frame first and a decodable chain; after a size stop, later deltas are dropped.
        if (!this.chunks.length && chunk.type !== "key") return;
        this.bytes += data.length;
        this.chunks.push({ data, timestampMs: chunk.timestamp / 1000, key: chunk.type === "key" });
      },
      error: (error) => {
        this.failure ??= error as Error;
      },
    });
    this.encoder.configure({
      codec: "vp8",
      width: w,
      height: h,
      bitrate: VIDEO_LIMITS.bitrate,
      framerate: 30,
      latencyMode: "realtime",
    });
  }

  /** Finish encoding and build the WebM file; `endSeconds` extends the duration to the stop time. */
  async finish(endSeconds?: number): Promise<{ bytes: Uint8Array; summary: VideoSummary }> {
    await this.queue;
    const endMs =
      endSeconds !== undefined && this.frames
        ? Math.min(Math.round(endSeconds * 1000), VIDEO_LIMITS.durationMs)
        : undefined;
    let repeatMs: number | undefined;
    if (
      endMs !== undefined &&
      endMs - HOLD_MS > this.lastMs &&
      !this.failure &&
      this.limit !== "size" &&
      this.frames < VIDEO_LIMITS.frames &&
      this.encoder?.state === "configured"
    ) {
      repeatMs = endMs - HOLD_MS;
      try {
        this.encodeCanvas(repeatMs);
      } catch (error: unknown) {
        this.failure ??= error instanceof Error ? error : new Error(String(error));
      }
    }
    try {
      if (this.encoder && this.encoder.state === "configured") await this.encoder.flush();
    } finally {
      this.close();
    }
    if (this.failure) throw this.failure;
    if (!this.chunks.length || !this.canvas) throw new Error("No video frames were captured");
    const lastMs = this.chunks.at(-1)?.timestampMs ?? 0;
    const durationMs = endMs === undefined ? lastMs + HOLD_MS : Math.max(lastMs, endMs);
    const bytes = muxWebm({
      width: this.canvas.width,
      height: this.canvas.height,
      codec: "V_VP8",
      durationMs,
      chunks: this.chunks,
    });
    return {
      bytes,
      summary: {
        frames: this.chunks.length - (this.chunks.at(-1)?.timestampMs === repeatMs ? 1 : 0),
        dropped: this.dropped,
        durationMs,
        width: this.canvas.width,
        height: this.canvas.height,
        bytes: bytes.length,
        ...(this.limit ? { limit: this.limit } : {}),
      },
    };
  }

  /** Stop without a file; idempotent. */
  close(): void {
    if (this.encoder && this.encoder.state !== "closed") this.encoder.close();
  }
}
