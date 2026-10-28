/**
 * The composer of one session (voice parts include the narration bar,
 * dictation strip, mic button, caret reporting): queued turns and the session's Loops above it, the run bar
 * while a turn runs, inserted context and image chips, the text box, the tool row and the hint line (the shortcut to
 * a waiting prompt while a card waits). The
 * top rule's colour shows the access mode. Loaded lazily (`LazyComposer.tsx`) so the main bundle does
 * not grow.
 * Keys (`model/keys.ts`): Enter sends (steers while a turn runs), Shift+Enter adds a line, Tab queues
 * while a turn runs, the `/` menu takes arrows / Enter / Tab / Escape. Picker, paste and drop route files
 * through the picture/document matrix. A very long text paste becomes a chip (`PasteChips.tsx`); Backspace
 * right after it removes it, and the full text is sent.
 * Depends on: AppContext, the composer store and its actions, the composer model, the parts and menus.
 * Called by: LazyComposer (the content area).
 * The same composer serves the new-chat view (`sessionId` = `NEW_CHAT`, no session yet): its picks are held
 * until the first send, there is no `/` menu, run bar or queue until the chat has started, documents wait in the draft, and
 * Send starts the session (`state/composer.ts`). Any composer takes the keyboard when asked (`state/composerFocus`).
 * The right corner: the mic, then Send only while there is something to send; Voice chat while the box is empty, and always while the session is in voice chat (its strip sits above the text box).
 * Invariants: render and event wiring only: every decision is in the model or the store.
 * A missing or failed Mistral key disables Send and submission shortcuts, while drafts stay editable.
 */
import { TEXT, term, wording } from "../../vocabulary";
import { formatKeys } from "../../state/shortcuts";
import { GO_TO_PROMPT_KEYS, goToWaitingPrompt, promptsOf, waitingPrompts } from "../ask/waiting";
import { type ReactElement, useEffect, useMemo, useRef } from "react";

import { Button } from "../../components/Controls";
import { droppedItems, runDrop } from "../files/drop";
import { fileItems } from "../files/attachments";
import { registerComposerFocus } from "../../state/composerFocus";
import { NEW_CHAT } from "../../state/newChatKey";
import { useApp } from "../shell/AppContext";
import { useKeyReadiness } from "../shell/KeyNotice";
import { useHeldBy } from "../sessions/SessionHeldNote";
import { useCoreState, useShellState } from "../shell/hooks";
import { ChipsRow, QueueList, RetryRow, RunBar } from "./ComposerParts";
import { openCodeRef } from "../files/openCodeRef";
import { DocPreview } from "./DocPreview";
import { DocumentChips } from "./DocumentChips";
import { PasteChips, usePasteFold } from "./PasteChips";
import { SlashMenu } from "./ComposerMenus";
import { FileMentionMenu } from "./FileMentionMenu";
import { useFileMention } from "./useFileMention";
import { LoopsStrip } from "./LoopsStrip";
import { useAutoHeight } from "./autoHeight";
import { ComposerTools } from "./ComposerTools";
import { NONE, useComposerState } from "./hooks";
import { configuredModeId, currentModeId, modeOf, offeredModes } from "./model/access";
import { keyAction } from "./model/keys";
import { currentPick, modelChoices, modelLabel, newChatPick } from "./model/models";
import { pendingProtectedChanges } from "../transcript/model/protectedChange";
import { lastTurnProblem, queuedTurns, runningTurnId, waitingForInput } from "./model/session";
import { type MenuItem, menuItems } from "./model/slash";
import { RewindDialog } from "./RewindDialog";
import { DictationStrip } from "../voice/DictationStrip";
import { MicButton } from "../voice/MicButton";
import { NarrationBar } from "../voice/NarrationBar";
import { useDictationCaret, useVoiceChat } from "../voice/hooks";
import { VoiceChatButton } from "../voice/VoiceChatButton";
import { VoiceChatStrip } from "../voice/VoiceChatStrip";

/** Most lines the text box grows to (wrapped lines count) before it scrolls. */
const MAX_ROWS = 8;

/** The composer for `sessionId`. */
export default function Composer({ sessionId }: { sessionId: string }): ReactElement | null {
  const app = useApp();
  const { composer, core, documents, market } = app;
  const key = useKeyReadiness();
  const readOnly = useHeldBy(sessionId) !== null;
  const heldKind = useCoreState((s) => s.heldKind[sessionId]);
  const entry = useCoreState((s) => s.sessions[sessionId]);
  const promptWaiting = useCoreState((s) => waitingPrompts(s).length > 0);
  const sessionPrompts = useCoreState((s) => promptsOf(s, sessionId).length);
  const sessionWaiting = sessionPrompts > 0;
  const draft = useComposerState((s) => s.drafts[sessionId] ?? "");
  const context = useComposerState((s) => s.context[sessionId] ?? NONE);
  const images = useComposerState((s) => s.images[sessionId] ?? NONE);
  const sending = useComposerState((s) => s.sending[sessionId] === true);
  const lobbyStarting = useComposerState((s) => s.sending[NEW_CHAT] === true);
  const compacting = useComposerState((s) => s.compacting[sessionId] === true);
  const note = useComposerState((s) => s.notes[sessionId] ?? null);
  const reloadFailed = useComposerState((s) => s.pluginReloadFailed[sessionId] === true);
  // Sticky provenance marks (composerStore): the Debug API's is a label; the rewind's has the way out.
  const mark = useComposerState((s) =>
    (s.drafts[sessionId] ?? "") === ""
      ? null
      : s.draftBy[sessionId] === "debug-api"
        ? "api"
        : s.literal[sessionId] === true
          ? "back"
          : null,
  );
  const skills = useComposerState((s) => s.skills[sessionId]);
  const menu = useComposerState((s) => (s.menu?.sessionId === sessionId ? s.menu.kind : null));
  const slashIndex = useComposerState((s) => s.slashIndex);
  const editing = useComposerState((s) => (s.editing?.sessionId === sessionId ? s.editing : null));
  const config = useComposerState((s) => s.config);
  const defaultReasoning = useComposerState((s) => s.defaultReasoning);
  const actions = composer.actions;
  const isNew = sessionId === NEW_CHAT;
  const newFolder = useShellState((s) => s.newFolder);
  const held = useComposerState((s) => s.newChat);
  const box = useRef<HTMLTextAreaElement>(null);
  const reportCaret = useDictationCaret(sessionId, box);
  const cwd = entry?.session["cwd"];
  const project = isNew ? newFolder : typeof cwd === "string" ? cwd : null;
  const mention = useFileMention(sessionId, draft, project, box);
  useAutoHeight(box, draft, MAX_ROWS);
  const paste = usePasteFold(sessionId, box);
  const voiceOn = useVoiceChat((s) => s.on && s.owner === sessionId);

  const state = entry?.state ?? null;
  const session = entry?.session;
  const activeTurnId = runningTurnId(state, session);
  const running = activeTurnId !== null;
  const inputWaiting = waitingForInput(state, activeTurnId);
  const queued = useMemo(() => queuedTurns(state), [state]);
  // The `/` menu lists the session's commands and skills: the new-chat view has none until its chat starts.
  const items = useMemo(
    () => (isNew ? null : menuItems(draft, skills ?? NONE)),
    [isNew, draft, skills],
  );
  const runtime = entry?.runtime;
  const models = useMemo(
    () => modelChoices(isNew ? (config ?? undefined) : runtime),
    [isNew, runtime, config],
  );
  // New chat: the held picks, else what the configuration says a new session gets.
  const pick = isNew
    ? newChatPick(config ?? undefined, held, defaultReasoning)
    : currentPick(runtime, session);
  // New chat: the held pick, else the mode the configuration gives a new session (the chip said "Access").
  const mode = modeOf(
    isNew
      ? (held.mode ?? configuredModeId(config ?? undefined))
      : currentModeId(runtime, session, entry?.accessMode),
  );

  // Load the action code now, so the first Enter does not wait for its chunk.
  useEffect(() => {
    actions?.preload();
  }, [actions]);
  useEffect(() => {
    if (items !== null && skills === undefined) void actions?.loadSkills(sessionId);
  }, [actions, items, skills, sessionId]);
  useEffect(
    () =>
      registerComposerFocus(sessionId, () => {
        const input = box.current;
        if (input === null) return;
        input.focus();
        input.setSelectionRange(input.value.length, input.value.length);
      }),
    [sessionId],
  );
  // Every composer resolves all three chips at entry; model menus also refresh their available choices.
  useEffect(() => {
    // The first-send action owns admission of held picks. Its newly selected composer must not
    // apply the global preference in parallel; unrelated fresh chats initialize when it finishes.
    if (!isNew && entry?.localOnly === true && lobbyStarting) return;
    void actions?.prepareSession(sessionId);
  }, [actions, sessionId, isNew, newFolder, lobbyStarting, entry?.localOnly]);

  if (actions === null || core === null) return null;
  const hasContent = draft.trim() !== "" || context.length > 0 || images.length > 0;
  const status =
    pendingProtectedChanges(state?.["history"]).length > 0
      ? "Waiting for you."
      : inputWaiting
        ? "Vibe is waiting for input."
        : compacting
          ? "Compacting context…"
          : queued.length > 0
            ? `${String(queued.length)} queued.`
            : "";
  const problem = running ? null : lastTurnProblem(state);
  const pickSlash = (item: MenuItem) => {
    if (item.kind === "manage") {
      composer.setDraft(sessionId, "");
      void market?.actions.showTab("skills");
      return;
    }
    if (item.kind === "skill") {
      composer.setDraft(sessionId, `/${item.skill.name} `);
      return;
    }
    if (key !== null) return;
    composer.setDraft(sessionId, "");
    void actions.runCommand(sessionId, item.row.name, "");
  };

  return (
    <div className="composer-wrap" data-session-id={sessionId}>
      <NarrationBar />
      <QueueList
        items={queued}
        editing={editing}
        onEdit={(item) => {
          composer.setEditing({ sessionId, queueItemId: item.id, text: item.text });
        }}
        onChange={(text) => {
          if (editing !== null) composer.setEditing({ ...editing, text });
        }}
        onSave={() => {
          if (editing === null) return;
          void actions.replaceQueued(sessionId, editing.queueItemId, editing.text);
        }}
        onCancel={() => {
          composer.setEditing(null);
        }}
        onRemove={(id) => void actions.removeQueued(sessionId, id)}
      />
      {problem === null ? null : (
        <RetryRow
          problem={problem}
          disabled={key !== null}
          onRetry={() => void actions.retry(sessionId, "")}
        />
      )}
      {isNew ? null : <LoopsStrip sessionId={sessionId} running={running} mode={mode} />}
      <div
        data-testid="composer.composer-on-drag-over-surface-1"
        className={`composer lvl-${mode?.level ?? "neutral"}${running ? " running" : ""}`}
        // the toast stack keeps the whole composer clear (Send, the mic, the tools row), not only the
        // focused text field: on the taller new-chat view a bottom-right toast covered Send.
        data-toast-avoid="composer"
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes("Files")) e.preventDefault();
        }}
        onDrop={(e) => {
          // every dropped file is routed by type (picture, document, refused with a reason).
          const items = droppedItems(e.dataTransfer);
          if (items.length === 0) return;
          e.preventDefault();
          void runDrop(app, sessionId, items, "ui");
        }}
      >
        {mention.mention !== null ? (
          <FileMentionMenu
            rows={mention.rows}
            index={mention.index}
            query={mention.mention.query}
            project={project}
            loading={mention.loading}
            error={mention.error}
            onPick={mention.pick}
            onHover={mention.hover}
          />
        ) : items === null ? null : (
          <SlashMenu
            items={items}
            index={Math.min(slashIndex, items.length - 1)}
            query={draft.slice(1)}
            onPick={pickSlash}
            onHover={composer.setSlashIndex}
          />
        )}
        {running || compacting || sessionWaiting ? (
          <RunBar
            status={
              sessionWaiting
                ? wording(sessionPrompts > 1 ? TEXT.askWaitingMany : TEXT.askWaitingHint, {
                    count: String(sessionPrompts),
                    shortcut: formatKeys(GO_TO_PROMPT_KEYS),
                  })
                : status
            }
            waiting={sessionWaiting}
            onShowPrompt={() => {
              goToWaitingPrompt(app, sessionId);
            }}
            inputWaiting={inputWaiting}
            onStop={() => void actions.stop(sessionId)}
          />
        ) : null}
        <DocumentChips sessionId={sessionId} />
        <ChipsRow
          onOpenRef={(ref) => void openCodeRef(app, sessionId, ref)}
          context={context}
          images={images}
          onRemoveContext={(id) => {
            composer.removeContext(sessionId, id);
          }}
          onRemoveImage={(id) => {
            composer.removeImage(sessionId, id);
          }}
          onPreview={(contextId) => {
            documents?.actions.preview({ sessionId, contextId });
          }}
        />
        <PasteChips sessionId={sessionId} onSwap={paste.swap} />
        <DictationStrip sessionId={sessionId} />
        <VoiceChatStrip sessionId={sessionId} />
        <label className="sr" htmlFor={`composer-input-${sessionId}`}>
          Message
        </label>
        <textarea
          data-testid="composer.composer-set-draft"
          ref={box}
          onSelect={() => {
            reportCaret();
            mention.reportCaret();
          }}
          id={`composer-input-${sessionId}`}
          title="Message"
          aria-label="Message"
          className="composer-input"
          rows={1}
          readOnly={readOnly}
          placeholder={
            readOnly && heldKind === "terminal"
              ? "Read-only while this session is open in the terminal"
              : context.length > 0
                ? "Tell Vibe what to do with this"
                : isNew
                  ? `${term("workNames", "chat").tooltip}: ask Vibe to build, fix, or explain something`
                  : "Ask Vibe to build, fix, or explain something"
          }
          value={draft}
          onChange={(e) => {
            composer.setDraft(sessionId, e.target.value);
            reportCaret();
            mention.reportCaret();
          }}
          onPaste={(e) => {
            const files = [...e.clipboardData.files];
            if (files.length === 0) {
              if (!readOnly) paste.onPaste(e);
              return;
            }
            e.preventDefault();
            void runDrop(app, sessionId, fileItems(files), "ui");
          }}
          onKeyDown={(e) => {
            if (readOnly) return;
            if (paste.onKey(e)) return;
            if (
              mention.key({
                key: e.key,
                shiftKey: e.shiftKey,
                ctrlKey: e.ctrlKey,
                altKey: e.altKey,
                metaKey: e.metaKey,
                isComposing: e.nativeEvent.isComposing,
              })
            ) {
              e.preventDefault();
              return;
            }
            const action = keyAction(
              {
                key: e.key,
                shiftKey: e.shiftKey,
                ctrlKey: e.ctrlKey,
                altKey: e.altKey,
                metaKey: e.metaKey,
                isComposing: e.nativeEvent.isComposing,
              },
              { running, slashOpen: items !== null && items.length > 0, hasContent },
            );
            if (action === null) return;
            e.preventDefault();
            const count = items?.length ?? 0;
            switch (action) {
              case "slashNext":
                composer.setSlashIndex(Math.min(slashIndex + 1, count - 1));
                return;
              case "slashPrev":
                composer.setSlashIndex(Math.max(slashIndex - 1, 0));
                return;
              case "slashPick": {
                const item = items?.[Math.min(slashIndex, count - 1)];
                if (item !== undefined) pickSlash(item);
                return;
              }
              case "slashClose":
                composer.setDraft(sessionId, "");
                return;
              default:
                if (key === null) void actions.send(sessionId, action);
            }
          }}
        />
        <ComposerTools
          {...(config === null
            ? {}
            : {
                defaultAccessLabel: modeOf(configuredModeId(config))?.label ?? TEXT.accessUnknown,
              })}
          mode={mode}
          modes={offeredModes(runtime)}
          modelLabel={modelLabel(models, pick.alias)}
          effort={pick.effort}
          alias={pick.alias}
          models={models}
          menu={menu}
          canSend={hasContent}
          blockedReason={readOnly ? "Read-only here." : (key?.message ?? null)}
          sending={sending}
          running={running}
          onMenu={(kind) => {
            void actions.openMenu(sessionId, kind);
          }}
          onAccess={(id, trustedClick) => void actions.setAccess(sessionId, id, trustedClick)}
          onModel={(alias) => void actions.pickModel(sessionId, alias)}
          onEffort={(level) => void actions.pickModel(sessionId, undefined, level)}
          onFiles={(files) => void runDrop(app, sessionId, fileItems(files), "ui")}
          onSend={() => {
            if (!readOnly && key === null) void actions.send(sessionId, "send");
          }}
          mic={<MicButton sessionId={sessionId} disabled={readOnly} />}
          voiceChat={<VoiceChatButton sessionId={sessionId} disabled={readOnly} />}
          voiceOn={voiceOn}
        />
      </div>
      {mark === "api" && (
        <p className="hint composer-mark" role="status" data-draft-by="debug-api">
          Typed by an external client (Debug API).
        </p>
      )}
      {mark === "back" && (
        <p className="hint composer-mark" role="status">
          Put back in the message box: Sent as text.{" "}
          <Button
            data-testid="composer.composer-adopt"
            type="button"
            appearance="compact"
            title="Use the restored message as my command"
            aria-label="Use the restored message as my command"
            onClick={() => {
              composer.adopt(sessionId);
            }}
          >
            Use as my command
          </Button>
        </p>
      )}
      {note === null || key !== null ? (
        <p className="hint">
          {key !== null ? (
            "You can keep typing · Shift+Enter for a new line"
          ) : promptWaiting ? (
            // Learnable by sight while a card waits.
            wording(TEXT.askWaitingHint, { shortcut: formatKeys(GO_TO_PROMPT_KEYS) })
          ) : isNew ? (
            "Enter to send: the chat starts in the folder above · Shift+Enter for a new line"
          ) : (
            <>
              {running ? "Enter to steer · Tab to queue" : "Enter to send"} · Shift+Enter for a new
              line ·{" "}
              <span
                title="Type / to choose commands and skills; arrows to select, Enter or Tab to choose, Escape to close"
                aria-label="Type / to choose commands and skills; arrows to select, Enter or Tab to choose, Escape to close"
              >
                / for commands and skills
              </span>{" "}
              ·{" "}
              <span
                title="Type ! followed by a shell command, then Enter to run it; with attachments it sends a message instead"
                aria-label="Type ! followed by a shell command, then Enter to run it; with attachments it sends a message instead"
              >
                ! runs a shell command
              </span>
            </>
          )}
        </p>
      ) : (
        <p className="hint composer-note" role="status">
          {note}
          {reloadFailed ? (
            <span className="composer-plugin-reload">
              <Button
                shape="outlined"
                data-testid="composer.retry-plugin-refresh"
                disabled={sending || compacting || running || sessionWaiting}
                onClick={() => void actions.retryPluginRefresh(sessionId)}
              >
                Retry plugin refresh
              </Button>
              <Button
                shape="outlined"
                data-testid="composer.start-with-new-plugins"
                onClick={() => void app.sessions?.startWithPlugins(sessionId)}
              >
                Start a new chat with the new plugins
              </Button>
            </span>
          ) : null}
        </p>
      )}
      <RewindDialog />
      <DocPreview />
    </div>
  );
}
