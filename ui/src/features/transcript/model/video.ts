/** Reads the optional core-owned video projection. Never infers a preview handle from tool output or a path. */
import { arr, num, obj, str, type Obj } from "./entries";
import { VIDEO_ENABLED } from "../../../state/videoEnabled";

/** Actual presentation time, rounded only for the visible millisecond label. */
export function videoTime(ns: number): string {
  const ms = Math.floor(ns / 1_000_000);
  return `${String(Math.floor(ms / 60000))}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}`;
}

/** Feature-off snapshots have no projection and leave the existing transcript unchanged. */
export function videoView(entry: Obj) {
  if (!VIDEO_ENABLED) return null;
  const video = obj(entry["vwVideo"]);
  const frames = num(video?.["selected_frames"]) ?? num(video?.["plannedFrames"]);
  const images = num(video?.["imageCount"]);
  if (video === null || frames === null || images === null || frames < 0 || frames > 24)
    return null;
  const scan = obj(video["scan"]);
  const start = num(scan?.["start_ms"]);
  const end = num(scan?.["end_ms"]);
  const step = num(scan?.["step_ms"]);
  const omitted = num(scan?.["omitted_changes"]) ?? 0;
  const coverage =
    start === null || end === null || step === null
      ? null
      : `Scanned ${videoTime(start * 1_000_000)}–${videoTime(end * 1_000_000)} · every ${String(step)} ms${omitted > 0 ? ` · ${String(omitted)} ${omitted === 1 ? "cut" : "cuts"} omitted` : ""}`;
  const prefix =
    video["frameLimit"] === true && video["selected_frames"] === undefined ? "Up to " : "";
  return {
    coverage,
    noChanges: scan !== null && num(scan["detected_changes"]) === 0,
    count: `${prefix}${String(frames)} ${frames === 1 ? "frame" : "frames"} · ${String(images)} ${images === 1 ? "image" : "images"}`,
    error: str(video["error"]),
    source: str(video["source"]) ?? "Video",
    metadata: `${arr(video["source_size"])
      .filter((v): v is number => typeof v === "number")
      .join(" × ")} · ${String(num(video["rotation_applied"]) ?? 0)}° · ${arr(
      video["pixel_aspect_ratio"],
    )
      .filter((v): v is number => typeof v === "number")
      .join(":")}`,
    frames: arr(video["frames"])
      .slice(0, 24)
      .flatMap((raw) => {
        const frame = obj(raw);
        const index = num(frame?.["index"]);
        const id = num(frame?.["frame_id"]);
        const time = num(frame?.["actual_ns"]);
        const file = str(frame?.["file"]);
        return index === null ||
          id === null ||
          time === null ||
          file === null ||
          !file.startsWith("video:")
          ? []
          : [{ index, id, time, file }];
      })
      .sort((a, b) => a.index - b.index),
  };
}
