/** The trusted main webview's VP8 encoder for guarded built-in browser screenshots. */
import { Channel, invoke } from "@tauri-apps/api/core";

import { WebmRecorder } from "./video-encoder";

type VideoRequest = {
  id: number;
  recording: string;
  kind: "start" | "frame" | "finish" | "cancel";
  png?: string;
  atSeconds?: number;
};
type VideoReply = {
  id: number;
  ok: boolean;
  supported?: boolean;
  bytes?: string;
  frames?: number;
  durationMs?: number;
  error?: string;
};

const recordings = new Map<string, WebmRecorder>();
const inFlight = new Map<string, Promise<void>>();

/** Whether this webview can encode guarded browser frames as VP8 video. */
export async function supportedVideoConfig(): Promise<boolean> {
  if (
    typeof VideoEncoder === "undefined" ||
    typeof VideoEncoder.isConfigSupported !== "function" ||
    typeof OffscreenCanvas === "undefined" ||
    typeof createImageBitmap !== "function" ||
    typeof VideoFrame === "undefined"
  )
    return false;
  try {
    if (!new OffscreenCanvas(2, 2).getContext("2d")) return false;
    const result = await VideoEncoder.isConfigSupported({
      codec: "vp8",
      width: 640,
      height: 480,
      bitrate: 2_000_000,
      framerate: 30,
      latencyMode: "realtime",
    });
    return result.supported === true;
  } catch {
    return false;
  }
}

function encodeBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(binary);
}

async function answer(request: VideoRequest): Promise<VideoReply> {
  const { id, recording, kind } = request;
  if (kind === "start") {
    if (!(await supportedVideoConfig())) return { id, ok: false, supported: false };
    if (recordings.has(recording)) return { id, ok: false, error: "Recording already started" };
    if (recordings.size >= 4) return { id, ok: false, error: "Too many recordings" };
    recordings.set(recording, new WebmRecorder());
    return { id, ok: true, supported: true };
  }
  const recorder = recordings.get(recording);
  if (!recorder) return { id, ok: false, error: "Recording ended" };
  if (kind === "cancel") {
    recorder.close();
    recordings.delete(recording);
    return { id, ok: true };
  }
  if (kind === "frame") {
    if (typeof request.png !== "string" || typeof request.atSeconds !== "number")
      return { id, ok: false, error: "Invalid frame" };
    await recorder.add(request.png, request.atSeconds);
    return { id, ok: true };
  }
  try {
    const result = await recorder.finish(request.atSeconds);
    return {
      id,
      ok: true,
      bytes: encodeBase64(result.bytes),
      frames: result.summary.frames,
      durationMs: result.summary.durationMs,
    };
  } finally {
    recordings.delete(recording);
  }
}

/** Subscribe once per main document. The capability grants this bridge only to the main webview. */
export async function startBrowserVideo(): Promise<void> {
  const channel = new Channel<VideoRequest>();
  channel.onmessage = (request) => {
    // A slow bitmap decode must finish before this recording's Stop or Cancel. Other chats keep encoding.
    const previous = inFlight.get(request.recording) ?? Promise.resolve();
    const next = previous
      .catch(() => undefined)
      .then(() => answer(request))
      .catch((error: unknown): VideoReply => ({
        id: request.id,
        ok: false,
        error: error instanceof Error ? error.message : "Video encoding failed",
      }))
      .then(async (reply) => {
        await invoke("browser_video_reply", { reply });
      });
    inFlight.set(request.recording, next);
    void next
      .finally(() => {
        if (inFlight.get(request.recording) === next) inFlight.delete(request.recording);
      })
      .catch(() => undefined);
  };
  await invoke("browser_video_subscribe", { requests: channel });
}
