/**
 * The composer's tool row: attach files (the OS file dialog through
 * `<input type="file">`: the webview gets the bytes of the files the user picked, never a path),
 * the access, model and thinking selectors, and in the right corner the dictation mic, always shown, directly left of one action slot (`ActionSlot`) whose face is
 * Send while there is something to send and Voice chat while the box is empty; while the session is in
 * voice chat the slot is Voice chat whatever the box holds (no Send button at all then).
 * A missing-key reason takes priority: the slot shows disabled Send with that reason as its tooltip.
 * Depends on: ComposerMenus, Icon, the composer model types. Called by: Composer.
 * Invariants: render only; selectors close on Escape or a pick; the slot keeps its place and
 * size whichever face it shows (nothing moves sideways), and a hidden face is out of the tab order and the
 * accessibility tree; the composer guards Enter with the same missing-key reason.
 */
import { type ReactElement, type ReactNode, useRef } from "react";

import { Icon } from "../../components/Icon";
import { ATTACH_ACCEPT, ATTACH_LABEL } from "../files/attachments";
import { AccessMenu, ModelMenu } from "./ComposerMenus";
import type { AccessMode } from "./model/access";
import type { ModelChoice } from "./model/models";

/** Props of the tool row. */
export interface ToolsProps {
  mode: AccessMode | null;
  modes: readonly AccessMode[];
  defaultAccessLabel?: string;
  modelLabel: string;
  effort: string | null;
  alias: string | null;
  models: readonly ModelChoice[];
  menu: "access" | "model" | null;
  canSend: boolean;
  /** Missing-key explanation; Send stays visible and disabled even for an empty draft. */
  blockedReason?: string | null;
  sending: boolean;
  running: boolean;
  onMenu: (menu: "access" | "model" | null) => void;
  onAccess: (id: string, trustedClick: boolean) => void;
  onModel: (alias: string) => void;
  onEffort: (level: string) => void;
  onFiles: (files: File[]) => void;
  /** The microphone button; absent: none. */
  mic?: ReactNode;
  onSend: () => void;
  /** Voice chat: the slot's face while the box is empty; absent: the slot stays empty. */
  voiceChat?: ReactNode;
  /** The session is in voice chat: the slot shows Voice chat only, never Send (Enter still sends). */
  voiceOn?: boolean;
}

/** The faces of the action slot. */
type SlotFace = "send" | "voiceChat";

/**
 * The composer's one action slot: Send and Voice chat share it. Both faces stay
 * mounted in the same box and cross-fade (`feel.css`, instant with reduced motion); the hidden one is inert and
 * aria-hidden, so it is neither tabbable nor announced.
 */
function ActionSlot(p: {
  face: SlotFace;
  running: boolean;
  sending: boolean;
  onSend: () => void;
  blockedReason?: string | null;
  voiceChat?: ReactNode;
}): ReactElement {
  const label = p.running ? "Steer the running turn (Enter)" : "Send (Enter)";
  const sendShown = p.face === "send";
  return (
    <span
      className="cslot"
      data-face={p.face}
      data-testid="composer-slot"
      title={p.blockedReason ?? undefined}
    >
      <span
        className="cslot-face"
        data-shown={sendShown}
        aria-hidden={!sendShown}
        inert={!sendShown}
      >
        <button
          data-testid="composer.composer-tools-send"
          type="button"
          className="send"
          aria-label={label}
          title={p.blockedReason ?? label}
          disabled={p.sending || p.blockedReason != null}
          onClick={p.onSend}
        >
          <Icon name="arrow-up" />
        </button>
      </span>
      <span
        className="cslot-face"
        data-shown={!sendShown}
        aria-hidden={sendShown}
        inert={sendShown}
        data-testid="voice-chat-face"
      >
        {p.voiceChat ?? null}
      </span>
    </span>
  );
}

/** The tool row under the text box. */
export function ComposerTools(p: ToolsProps): ReactElement {
  const picker = useRef<HTMLInputElement>(null);
  return (
    <div
      data-testid="composer.composer-tools-ctools"
      className="ctools"
      onKeyDown={(e) => {
        if (e.key === "Escape" && p.menu !== null) {
          e.stopPropagation();
          p.onMenu(null);
        }
      }}
    >
      <button
        data-testid="composer.composer-tools-click"
        type="button"
        className="ibtn"
        aria-label={ATTACH_LABEL}
        title={ATTACH_LABEL}
        onClick={() => picker.current?.click()}
      >
        <Icon name="paperclip" />
      </button>
      <input
        ref={picker}
        type="file"
        accept={ATTACH_ACCEPT}
        multiple
        hidden
        data-testid="composer.file-picker"
        onChange={(e) => {
          const files = [...(e.target.files ?? [])];
          e.target.value = "";
          if (files.length > 0) p.onFiles(files);
        }}
      />
      <AccessMenu
        {...(p.defaultAccessLabel === undefined ? {} : { defaultLabel: p.defaultAccessLabel })}
        modes={p.modes}
        current={p.mode?.id ?? null}
        {...(p.mode === null ? {} : { currentLabel: p.mode.label })}
        onPick={p.onAccess}
        onOpen={() => {
          p.onMenu("access");
        }}
        onClose={() => {
          p.onMenu(null);
        }}
      />
      <ModelMenu
        models={p.models}
        alias={p.alias}
        effort={p.effort}
        modelLabel={p.modelLabel}
        onOpen={() => {
          p.onMenu("model");
        }}
        onClose={() => {
          p.onMenu(null);
        }}
        onModel={(alias) => {
          p.onModel(alias);
          p.onMenu(null);
        }}
        onEffort={(level) => {
          p.onEffort(level);
          p.onMenu(null);
        }}
      />
      {/* The right corner: the mic, then the action slot (tab order = visual order). */}
      <span className="ccorner push-right">
        {p.mic ?? null}
        <ActionSlot
          face={p.blockedReason != null || (p.canSend && p.voiceOn !== true) ? "send" : "voiceChat"}
          blockedReason={p.blockedReason ?? null}
          running={p.running}
          sending={p.sending}
          onSend={p.onSend}
          {...(p.voiceChat === undefined ? {} : { voiceChat: p.voiceChat })}
        />
      </span>
    </div>
  );
}
