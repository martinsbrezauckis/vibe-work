/**
 * Microphone capture for dictation: the
 * webview turns the microphone into 16-bit mono PCM chunks of about 250 ms, which `state/dictation` sends
 * to the core. Voxtral realtime takes raw PCM only (no WebM/Opus), so MediaRecorder (ShellX's path) is not
 * used. The AudioContext asks for 16 kHz (Chromium, WebKit and WebKitGTK resample natively); when the
 * engine gives another rate, the chunks say so and the core resamples.
 * Capture runs in an AudioWorklet (`pcm-worklet.js`, loaded from the app's own origin).
 * Depends on: the Web Audio and media device APIs. Called by: state/dictation (through `PrepareCapture`).
 * Invariants: every path that fails after `getUserMedia` succeeded stops the tracks (the microphone light
 * goes off); `stop()` is idempotent; nothing leaves this module but
 * the chunks given to `onChunk`.
 */

/** A running capture. */
interface Capture {
  /** Stops the microphone and the audio graph; flushes the last partial chunk first. */
  stop(): void;
}

/** What opening the microphone needs. */
export interface CaptureOptions {
  /** A `deviceId` from Settings, or `null` for the system default. */
  deviceId: string | null;
  /** One chunk of 16-bit little-endian PCM at `sampleRate`. */
  onChunk: (pcm: Uint8Array) => void;
  /**
   * The system ended the microphone (unplugged, taken away, permission revoked; voice chat then turns
   * off with the reason). Not called for our own `stop()`.
   */
  onEnded?: () => void;
}

/**
 * The audio graph, ready but **without the microphone**: its rate is known before the core is asked to
 * start a stream (the core must know the rate), and the microphone opens only after the core said yes.
 */
interface PreparedCapture {
  /** The rate the chunks will be at. */
  readonly sampleRate: number;
  /** Opens the microphone (the OS permission prompt, if any, shows here). */
  open(options: CaptureOptions): Promise<Capture>;
  /** Releases the audio graph without opening the microphone. */
  discard(): void;
}

/** Prepares a capture (the function `state/dictation` is given; tests pass a fake). */
export type PrepareCapture = () => Promise<PreparedCapture>;

/** Why the microphone could not start, in words for the user. */
export function micErrorText(error: unknown): string {
  if (error instanceof Error && error.message.startsWith("This window")) return error.message;
  const name = error instanceof DOMException ? error.name : "";
  if (name === "NotAllowedError" || name === "SecurityError")
    return "Microphone access was denied. Allow it for Vibe Work in your system's privacy settings, then try again.";
  if (name === "NotFoundError" || name === "OverconstrainedError")
    return "No microphone was found. Connect one or pick another in Settings, Voice.";
  if (name === "NotReadableError")
    return "The microphone is busy or could not be opened by the system.";
  return "The microphone could not be started.";
}

/** Converts floats in -1..1 to 16-bit little-endian bytes. */
function floatToPcm16(samples: Float32Array): Uint8Array {
  const out = new Uint8Array(samples.length * 2);
  const view = new DataView(out.buffer);
  for (let i = 0; i < samples.length; i += 1) {
    const s = Math.max(-1, Math.min(1, samples[i] ?? 0));
    view.setInt16(i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  return out;
}

/** Collects float blocks and hands out PCM chunks of `size` samples. */
function chunker(size: number, onChunk: (pcm: Uint8Array) => void) {
  let buffer = new Float32Array(size);
  let filled = 0;
  return {
    push(block: Float32Array): void {
      let offset = 0;
      while (offset < block.length) {
        const take = Math.min(size - filled, block.length - offset);
        buffer.set(block.subarray(offset, offset + take), filled);
        filled += take;
        offset += take;
        if (filled === size) {
          onChunk(floatToPcm16(buffer));
          buffer = new Float32Array(size);
          filled = 0;
        }
      }
    },
    flush(): void {
      if (filled > 0) onChunk(floatToPcm16(buffer.subarray(0, filled)));
      filled = 0;
    },
  };
}

/**
 * The block tap: an AudioWorklet node feeding `onBlock`. Every webview Vibe Work supports has AudioWorklet
 * (WebView2, WKWebView, WebKitGTK); without it dictation says so instead of using the deprecated
 * ScriptProcessorNode.
 */
async function tap(
  context: AudioContext,
  onBlock: (block: Float32Array) => void,
): Promise<AudioNode> {
  if (!("audioWorklet" in context)) {
    throw new Error("This window cannot capture audio (no AudioWorklet).");
  }
  await context.audioWorklet.addModule(new URL("./pcm-worklet.js", import.meta.url).href);
  const node = new AudioWorkletNode(context, "vw-pcm-tap");
  node.port.onmessage = (e: MessageEvent<Float32Array>) => {
    onBlock(e.data);
  };
  return node;
}

/** The real capture: an AudioContext at 16 kHz with the tap, then `getUserMedia` on `open`. */
export const prepareCapture: PrepareCapture = async () => {
  const context = new AudioContext({ sampleRate: 16_000 });
  let deliver: (block: Float32Array) => void = () => undefined;
  let node: AudioNode;
  try {
    node = await tap(context, (block) => {
      deliver(block);
    });
  } catch (thrown) {
    void context.close().catch(() => undefined);
    throw thrown;
  }
  const rate = context.sampleRate;
  const release = () => void context.close().catch(() => undefined);
  return {
    sampleRate: rate,
    discard: release,
    async open({ deviceId, onChunk, onEnded }) {
      const stream = await navigator.mediaDevices
        .getUserMedia({
          audio: {
            ...(deviceId === null ? {} : { deviceId: { exact: deviceId } }),
            channelCount: 1,
            echoCancellation: true,
            noiseSuppression: true,
          },
        })
        .catch((thrown: unknown) => {
          release();
          throw thrown;
        });
      const chunks = chunker(Math.round(rate / 4), onChunk);
      deliver = (block) => {
        chunks.push(block);
      };
      try {
        if (context.state === "suspended") await context.resume();
        const source = context.createMediaStreamSource(stream);
        source.connect(node);
        // A tap only runs when connected to the destination; a gain of 0 keeps it silent.
        const mute = context.createGain();
        mute.gain.value = 0;
        node.connect(mute).connect(context.destination);
      } catch (thrown) {
        for (const track of stream.getTracks()) track.stop();
        release();
        throw thrown;
      }
      let stopped = false;
      for (const track of stream.getTracks()) {
        track.addEventListener("ended", () => {
          if (!stopped) onEnded?.();
        });
      }
      return {
        stop() {
          if (stopped) return;
          stopped = true;
          chunks.flush();
          deliver = () => undefined;
          for (const track of stream.getTracks()) track.stop();
          release();
        },
      };
    },
  };
};

/** The microphones the system lists (labels only after the user allowed the microphone once). */
export async function listMicrophones(): Promise<{ id: string; label: string }[]> {
  let devices: MediaDeviceInfo[];
  try {
    // `mediaDevices` is missing outside a secure context (typed as always present).
    devices = await navigator.mediaDevices.enumerateDevices();
  } catch {
    return [];
  }
  return devices
    .filter((d) => d.kind === "audioinput" && d.deviceId !== "" && d.deviceId !== "default")
    .map((d, i) => ({ id: d.deviceId, label: d.label || `Microphone ${String(i + 1)}` }));
}
