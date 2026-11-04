/** Saved video cells beside their tool row; reuses Files reads, the image viewer and shared buttons. */
import { useEffect, useState } from "react";
import { Button } from "../../../components/Controls";
import { useFileSessionId } from "../../files/FileRef";
import { useApp } from "../../shell/AppContext";
import type { Obj } from "../model/entries";
import { videoTime, videoView } from "../model/video";

function Frame({ file, time, index }: { file: string; time: number; index: number }) {
  const { files, shell } = useApp();
  const session = useFileSessionId();
  const [url, setUrl] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    let live = true;
    if (session !== null && files !== null)
      void files.image(session, file).then((value) => {
        if (live) {
          setUrl(value);
          setLoaded(true);
        }
      });
    return () => {
      live = false;
    };
  }, [files, session, file]);
  const stamp = videoTime(time);
  const label = `Open frame at ${stamp}`;
  return (
    <li data-frame-index={index}>
      <Button
        data-testid="transcript.video-frame-open"
        data-item-id={`${file}:${String(index)}`}
        appearance="file-reference"
        aria-label={label}
        title={label}
        onClick={() => {
          if (files === null || session === null) return;
          shell.actions.openTab("file");
          void files.actions.show(session, { file, display: `Frame at ${stamp}`, line: null });
        }}
      >
        {url === null ? (
          <span className="video-frame-placeholder">{loaded ? "Unavailable" : "Loading…"}</span>
        ) : (
          <img src={url} alt={`Frame at ${stamp}`} />
        )}
        <span className="ui-caption">{stamp}</span>
      </Button>
    </li>
  );
}

/** Only enabled-core rows get a count, failure label or frame strip. */
export function VideoFrames({ entry }: { entry: Obj }) {
  const video = videoView(entry);
  if (video === null) return null;
  return (
    <div className="video-review" data-video-review="">
      <p className="step-outcome" role="status">
        {video.count}
      </p>
      {video.coverage === null ? null : <p className="step-outcome">{video.coverage}</p>}
      {video.noChanges ? (
        <p className="step-outcome">No scene changes found at this sampling step.</p>
      ) : null}
      {video.error === null ? null : <p className="step-outcome">{video.error}</p>}
      {video.frames.length === 0 ? null : (
        <ol
          className="video-frames"
          aria-label={`Frames from ${video.source}`}
          title={video.metadata}
        >
          {video.frames.map((frame) => (
            <Frame key={frame.index} file={frame.file} time={frame.time} index={frame.index} />
          ))}
        </ol>
      )}
    </div>
  );
}
