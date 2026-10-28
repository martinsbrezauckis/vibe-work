/**
 * Message rows: the user's message (`.u`) and the assistant's answer (`.a-msg`).
 * - User: the text as typed (plain text, `userDisplayContent` when Vibe kept one), a "Loop" tag when a loop sent it, images the core
 *   provided as `data:` URLs (never `file://` or asset URLs; a file-path image is named
 *   only), context chips, a "steered" mark for a second message in a running turn, Copy, and
 *   Rewind to here / Branch from here through the composer's actions.
 * - Assistant: Markdown (`components/markdown`, no raw HTML, a single newline is a line break as in chat apps),
 *   `remend` while streaming, Copy, time.
 *   A message over `MESSAGE_CHARS` renders its start as Markdown; "Show all" shows the whole text as
 *   plain text (a 2 MB Markdown parse would freeze the window).
 * Adapted from ShellX.
 * Depends on: model/entries, model/text, Markdown, CopyButton, Time, the transcript store.
 * Called by: RowView.
 */
import { Button, Badge } from "../../../components/Controls";
import { type ReactElement, memo } from "react";

import { CopyButton } from "../../../components/CopyButton";
import { anchorProblem } from "../../composer/model/session";
import { Markdown } from "../../../components/markdown/Markdown";
import { FILE_REFS } from "../../files/FileRef";
import { useApp } from "../../shell/AppContext";
import { useTranscriptUi } from "../hooks";
import {
  type Obj,
  contextChips,
  createdAt,
  displayText,
  entryId,
  inProgress,
  loopMessage,
  messageImages,
  messageText,
  spokenMessage,
} from "../model/entries";
import { openCodeRef } from "../../files/openCodeRef";
import { gatherSources } from "../model/sources";
import { safeHead } from "../model/text";
import { Time } from "./Time";

/** Longest assistant message rendered as Markdown before "Show all". */
const MESSAGE_CHARS = 100_000;

/**
 * "Rewind to here" and "Branch from here": enabled only for messages Vibe can anchor on
 * (our own client message ids); otherwise disabled with the reason as the title. Text buttons like Copy (no
 * SVG inside transcript rows: the trusted-surface test forbids `svg` there).
 */
function MessageActions({
  entry,
  sessionId,
  text,
}: {
  entry: Obj;
  sessionId: string;
  text: string;
}) {
  const { composer } = useApp();
  const actions = composer.actions;
  if (actions === null) return null;
  const problem = anchorProblem(entry);
  const id = entryId(entry);
  const rewindLabel =
    problem === null ? "Rewind to here (restores files)" : `Rewind to here: ${problem}`;
  const branchLabel =
    problem === null ? "Branch from here in a new session" : `Branch from here: ${problem}`;
  return (
    <>
      <button
        data-testid="transcript.message-rows-open-rewind"
        type="button"
        className="copy-btn"
        aria-label={rewindLabel}
        title={rewindLabel}
        disabled={problem !== null}
        onClick={() => void actions.openRewind(sessionId, id, text)}
      >
        Rewind
      </button>
      <button
        data-testid="transcript.message-rows-branch"
        type="button"
        className="copy-btn"
        aria-label={branchLabel}
        title={branchLabel}
        disabled={problem !== null}
        onClick={() => void actions.branch(sessionId, id)}
      >
        Branch
      </button>
    </>
  );
}

/** The user's message. */
export const UserRow = memo(function UserRow({
  entry,
  steer,
  sessionId,
}: {
  entry: Obj;
  steer: boolean;
  sessionId: string;
}): ReactElement {
  const text = displayText(entry) ?? messageText(entry);
  const { images, fileImages } = messageImages(entry);
  const app = useApp();
  const context = contextChips(entry);
  const loop = loopMessage(entry);
  return (
    <div
      className={`u${steer ? " steer" : ""}`}
      data-entry-id={entryId(entry)}
      data-item-id={entryId(entry)}
    >
      <div className="acts2">
        <MessageActions entry={entry} sessionId={sessionId} text={messageText(entry)} />
        <CopyButton testId="transcript.message-rows-copybutton" text={text} label="Copy message" />
      </div>
      <div className="u-meta">
        {steer ? <Badge presentation="user-label">Steered</Badge> : null}
        {loop === null ? null : (
          <Badge
            presentation="loop-label"
            data-loop-id={loop}
            title={`Sent by a loop${loop === "" ? "" : ` (${loop})`} on its own: you did not type it`}
          >
            Loop
          </Badge>
        )}
        {spokenMessage(entry) ? (
          <Badge presentation="voice-label" title="Said in voice chat; the answer was read aloud">
            voice
          </Badge>
        ) : null}
        <Time ms={createdAt(entry)} />
      </div>
      {context.length > 0 ? (
        <div className="ctx-chips">
          {context.map((chip, i) =>
            chip.ref === undefined ? (
              <Badge key={i} presentation="context-label" title={chip.label}>
                {chip.label}
              </Badge>
            ) : (
              <Badge
                as="button"
                key={i}
                type="button"
                data-testid="transcript.message-rows-open-context"
                data-item-id={chip.label}
                presentation="context-label"
                title={`Open ${chip.label} in the File tab`}
                aria-label={`Open ${chip.label} in the File tab`}
                onClick={() => {
                  if (chip.ref !== undefined) void openCodeRef(app, sessionId, chip.ref);
                }}
              >
                {chip.label}
              </Badge>
            ),
          )}
        </div>
      ) : null}
      <div className="u-text">{text}</div>
      {images.length + fileImages.length > 0 ? (
        <div className="u-imgs">
          {images.map((image, i) => (
            <img key={i} src={image.src} alt={image.alt} className="u-img" />
          ))}
          {fileImages.map((name, i) => (
            <Badge key={`f${String(i)}`} presentation="image-label" title={name}>
              {name}
            </Badge>
          ))}
        </div>
      ) : null}
    </div>
  );
});

/** The "Sources (n)" chip of an answer that used web search; opens the side list. */
function SourcesChip({ rowKey, searches }: { rowKey: string; searches: readonly Obj[] }) {
  const { transcript } = useApp();
  const open = useTranscriptUi((s) => s.sources?.key === rowKey);
  const label = open ? "Hide sources for this answer" : "Show sources for this answer";
  const sources = gatherSources(searches);
  if (sources.length === 0) return null;
  return (
    <Badge
      data-testid="transcript.message-rows-show-sources"
      type="button"
      presentation="sources"
      as="button"
      aria-expanded={open}
      aria-label={label}
      title={label}
      data-sources-chip={rowKey}
      onClick={() => {
        transcript.showSources(open ? null : { key: rowKey, items: sources });
      }}
    >
      Sources ({String(sources.length)})
    </Badge>
  );
}

/** The assistant's answer. `searches`: the web searches before it (shown by the "Sources" chip). */
export const AssistantRow = memo(function AssistantRow({
  entry,
  searches,
  rowKey,
}: {
  entry: Obj;
  searches: readonly Obj[];
  rowKey: string;
}): ReactElement {
  const { transcript } = useApp();
  const id = entryId(entry);
  const key = `all:${id}`;
  const full = useTranscriptUi((s) => s.open[key] === true);
  const text = messageText(entry);
  const streaming = inProgress(entry);
  const big = text.length > MESSAGE_CHARS;
  return (
    <div className={`a-msg${streaming ? " streaming" : ""}`} data-entry-id={id} data-item-id={id}>
      {big && full ? (
        <pre className="out plain">{text}</pre>
      ) : (
        <Markdown
          source={big ? safeHead(text, MESSAGE_CHARS) : text}
          streaming={streaming}
          lineBreaks
          onOpenLink={(url) => {
            transcript.askLink(url);
          }}
          // File references: clickable once the core said they name a file the app may show.
          extension={FILE_REFS}
        />
      )}
      {big && !full ? (
        <Button
          data-testid="transcript.message-rows-toggle"
          type="button"
          appearance="expander"
          aria-label="Show full message as plain text"
          title="Show full message as plain text"
          onClick={() => {
            transcript.toggle(key, true);
          }}
        >
          Show all ({String(Math.round((text.length - MESSAGE_CHARS) / 1000))} KB more, as plain
          text)
        </Button>
      ) : null}
      <div className="a-meta">
        <Time ms={createdAt(entry)} />
        {streaming ? null : (
          <CopyButton
            testId="transcript.message-rows-copybutton-2"
            text={text}
            label="Copy message"
          />
        )}
        {streaming ? null : <SourcesChip rowKey={rowKey} searches={searches} />}
      </div>
    </div>
  );
});
