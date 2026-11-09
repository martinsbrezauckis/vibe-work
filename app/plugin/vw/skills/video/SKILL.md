---
name: video
description: Vibe Work's video_frames tool - review video frames at given seconds, with even spacing, at an interval or at scene changes, as single frames or contact sheets.
---

`video_frames` is on `mcp_vw` inside `run_typescript`; one program has 60 s for all of its tool calls:

```ts
const frames = await tools.mcp_vw.video_frames({"path":"clips/demo.mp4","selection":{"kind":"timestamps","times_ms":[1000,3500]},"output":"contact_sheet","max_edge":960});
```

Windows, macOS and Linux (Linux uses the system's GStreamer). The source is a regular local file up to 512 MiB and two hours long. Audio is not processed.

- Times are integer milliseconds (`1000` = one second). `timestamps` selects `times_ms`; `uniform` selects `count` evenly spaced frames; `interval` uses `every_ms` and `max_frames`; `scene_changes` compares small pictures at `step_ms`, with `max_frames` and at most 240 samples.
- Uniform, interval and scene selections use `start_ms` and exclusive `end_ms` within the video. One selection spans at most ten minutes and returns at most 24 frames. Scene changes return the earliest detected cuts and report sampled coverage and omitted cuts; events between samples or subtle cuts can be missed.
- `output` is `frame` (one selected frame), `contact_sheet` (one image of the selected frames) or `metadata` (no image). `max_edge` is 64..1280 pixels. Images share the user turn's image allowance.
- The result reports requested and actual presentation times and the planned frame/image counts. Unsupported formats/codecs, worker failures and limits have separate reported outcomes.
