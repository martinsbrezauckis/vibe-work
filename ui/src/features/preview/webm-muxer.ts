// Adapted from chrome-control/extension/src/adapter/webm-muxer.ts at int0115 for Vibe Work's trusted main webview.
// Minimal WebM (Matroska) writer for one VP8 video track; used by video-encoder.ts.
// The whole file is built in memory after encoding finishes, so every element has its exact size, the Segment
// carries its Duration and a Cues index (seekable in browsers and players). Timecodes are milliseconds.
// Layout: EBML header, Segment { Info, Tracks, Cluster*, Cues }. A cluster starts at every key frame and at least
// every 30 s, because SimpleBlock timecodes are signed 16-bit offsets from their cluster's timecode.

/** One encoded VP8 frame with its presentation time and key-frame flag. */
export type EncodedChunk = { data: Uint8Array; timestampMs: number; key: boolean };
/** Dimensions, duration, and frames for one VP8 WebM file. */
export type WebmInput = {
  width: number;
  height: number;
  codec: "V_VP8";
  durationMs: number;
  chunks: EncodedChunk[];
  app?: string;
};

const MAX_CLUSTER_SPAN_MS = 30_000;
const encoder = new TextEncoder();

/** EBML variable-length size: the shortest form whose all-ones value stays reserved for "unknown". */
function vintSize(size: number): Uint8Array {
  for (let length = 1; length <= 8; length++) {
    if (size < 2 ** (7 * length) - 1) {
      const out = new Uint8Array(length);
      let rest = size;
      for (let i = length - 1; i >= 0; i--) {
        out[i] = rest % 256;
        rest = Math.floor(rest / 256);
      }
      out[0] = (out[0] ?? 0) | (1 << (8 - length));
      return out;
    }
  }
  throw new RangeError("EBML element too large");
}
function idBytes(id: number): Uint8Array {
  const bytes: number[] = [];
  for (let rest = id; rest > 0; rest = Math.floor(rest / 256)) bytes.unshift(rest % 256);
  return Uint8Array.from(bytes);
}
function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}
function element(id: number, body: Uint8Array | Uint8Array[]): Uint8Array {
  const data = Array.isArray(body) ? concat(body) : body;
  return concat([idBytes(id), vintSize(data.length), data]);
}
function uint(id: number, value: number): Uint8Array {
  const bytes: number[] = [];
  let rest = Math.max(0, Math.floor(value));
  do {
    bytes.unshift(rest % 256);
    rest = Math.floor(rest / 256);
  } while (rest > 0);
  return element(id, Uint8Array.from(bytes));
}
function float64(id: number, value: number): Uint8Array {
  const data = new Uint8Array(8);
  new DataView(data.buffer).setFloat64(0, value);
  return element(id, data);
}
function text(id: number, value: string): Uint8Array {
  return element(id, encoder.encode(value));
}

function simpleBlock(chunk: EncodedChunk, clusterMs: number): Uint8Array {
  const header = new Uint8Array(4);
  header[0] = 0x81; // track number 1 as a one-byte vint
  new DataView(header.buffer).setInt16(1, Math.round(chunk.timestampMs - clusterMs));
  header[3] = chunk.key ? 0x80 : 0x00;
  return element(0xa3, [header, chunk.data]);
}

/** Build a complete, seekable WebM file. The first chunk must be a key frame; timestamps must not decrease. */
export function muxWebm(input: WebmInput): Uint8Array {
  const { chunks } = input;
  const first = chunks[0];
  if (!first?.key) throw new Error("A WebM video needs a key frame first");
  if (
    !Number.isInteger(input.width) ||
    !Number.isInteger(input.height) ||
    input.width < 1 ||
    input.height < 1
  )
    throw new Error("Invalid video size");
  for (let i = 1; i < chunks.length; i++)
    if ((chunks[i]?.timestampMs ?? 0) < (chunks[i - 1]?.timestampMs ?? 0))
      throw new Error("Timestamps must not decrease");
  const app = input.app ?? "Vibe Work";
  const header = element(0x1a45dfa3, [
    uint(0x4286, 1),
    uint(0x42f7, 1),
    uint(0x42f2, 4),
    uint(0x42f3, 8),
    text(0x4282, "webm"),
    uint(0x4287, 4),
    uint(0x4285, 2),
  ]);
  const info = element(0x1549a966, [
    uint(0x2ad7b1, 1_000_000),
    text(0x4d80, app),
    text(0x5741, app),
    float64(0x4489, Math.max(input.durationMs, chunks.at(-1)?.timestampMs ?? 0)),
  ]);
  const video = element(0xe0, [uint(0xb0, input.width), uint(0xba, input.height)]);
  const tracks = element(
    0x1654ae6b,
    element(0xae, [
      uint(0xd7, 1),
      uint(0x73c5, 1),
      uint(0x83, 1),
      uint(0x9c, 0),
      text(0x86, input.codec),
      video,
    ]),
  );
  // Clusters: positions are offsets from the first byte of the Segment's data, as Cues require.
  const clusters: Uint8Array[] = [];
  const cues: Uint8Array[] = [];
  let position = info.length + tracks.length;
  for (let start = 0; start < chunks.length;) {
    const clusterMs = Math.round(chunks[start]?.timestampMs ?? 0);
    let end = start + 1;
    while (
      end < chunks.length &&
      !chunks[end]?.key &&
      (chunks[end]?.timestampMs ?? 0) - clusterMs < MAX_CLUSTER_SPAN_MS
    )
      end++;
    const cluster = element(0x1f43b675, [
      uint(0xe7, clusterMs),
      ...chunks.slice(start, end).map((chunk) => simpleBlock(chunk, clusterMs)),
    ]);
    if (chunks[start]?.key)
      cues.push(
        element(0xbb, [
          uint(0xb3, clusterMs),
          element(0xb7, [uint(0xf7, 1), uint(0xf1, position)]),
        ]),
      );
    clusters.push(cluster);
    position += cluster.length;
    start = end;
  }
  const segment = element(0x18538067, [info, tracks, ...clusters, element(0x1c53bb6b, cues)]);
  return concat([header, segment]);
}
