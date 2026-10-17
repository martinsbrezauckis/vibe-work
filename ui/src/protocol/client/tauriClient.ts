/**
 * The real app's client: Tauri `invoke` for commands and a Tauri `Channel` for the event stream.
 * A Channel (ordered, built for streaming) is used instead of global events: Tauri's own docs say
 * events are not meant for throughput, and ten agents streaming is the design load. It also
 * needs no event permission in the capability.
 * Depends on: @tauri-apps/api, ./types. Called by: ./select.
 * Invariants: the commands called here are exactly the ones the capability grants
 * (`app/capabilities/main.json`); nothing else is invoked.
 */
import { Channel, invoke } from "@tauri-apps/api/core";

import type { Command, CommandResult, CoreEvent, PagesResult, SessionExportView } from "../core";
import {
  type CliSharingView,
  type CoreClient,
  type DocumentsPicked,
  type HostEvent,
  type TerminalChunkHandler,
  type TerminalStream,
  type UpdateView,
  type UiControlReply,
  type UiControlRequest,
  toCallError,
} from "./types";

async function call<T>(name: string, args: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args);
  } catch (thrown) {
    throw toCallError(thrown);
  }
}

/** True when running inside the Tauri webview. */
export function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The Tauri-backed client. */
export function createTauriClient(): CoreClient {
  return {
    kind: "tauri",
    execute: (command: Command, options) =>
      call<CommandResult>(
        "core_execute",
        options?.origin === "debug-api"
          ? { command, debugApiCapability: options.capability ?? "" }
          : { command },
      ),
    async subscribe(onEvent) {
      const channel = new Channel<CoreEvent>();
      channel.onmessage = onEvent;
      await call<null>("core_subscribe", { events: channel });
      return () => {
        channel.onmessage = () => undefined;
      };
    },
    uiControl: {
      next: (waitMs) => call<UiControlRequest | null>("ui_control_next", { waitMs }),
      async reply(reply: UiControlReply) {
        await call<boolean>("ui_control_reply", { reply });
      },
    },
    pickFolder: () => call<string | null>("pick_folder", {}),
    pickDocuments: () => call<DocumentsPicked>("pick_documents", {}),
    pickCertificate: () => call<string | null>("pick_certificate", {}),
    pickLibraryFiles: (libraryId: string) =>
      call<CommandResult>("pick_library_files", { libraryId }),
    exportPage: (sessionId: string, path: string) =>
      call<PagesResult | null>("export_page", { sessionId, path }),
    exportSession: (sessionId: string) =>
      call<SessionExportView | null>("export_session", { sessionId }),
    async revealFolder(path) {
      await call<null>("reveal_folder", { path });
    },
    async openLink(url) {
      await call<null>("open_link", { url });
    },
    async openMailLink(url) {
      await call<null>("open_mail_link", { url });
    },
    async notifyOs(notification) {
      await call<null>("notify_session", { request: notification });
    },
    async onHostEvent(onEvent) {
      const channel = new Channel<HostEvent>();
      channel.onmessage = onEvent;
      await call<null>("host_subscribe", { events: channel });
      return () => {
        channel.onmessage = () => undefined;
      };
    },
    cliSharing: (share) => call<CliSharingView>("cli_sharing", { share: share ?? null }),
    async setDebugApi(enabled) {
      await call<null>("debug_api_set", { enabled });
    },
    debugApiTokenFile: (rotate) => call<string>("debug_api_token_file", { rotate }),
    attachTerminal: attachTauriTerminal,
    updates: {
      async subscribe(onView) {
        const channel = new Channel<UpdateView>();
        channel.onmessage = onView;
        return call<boolean>("update_subscribe", { events: channel });
      },
      async check() {
        await call<null>("update_check", {});
      },
      async download(options) {
        await call<null>(
          "update_download",
          options?.origin === "debug-api" ? { debugApiCapability: options.capability ?? "" } : {},
        );
      },
      async install(options) {
        await call<null>(
          "update_install",
          options?.origin === "debug-api" ? { debugApiCapability: options.capability ?? "" } : {},
        );
      },
      async setAutoDownload(enabled) {
        await call<null>("update_set_auto_download", { enabled });
      },
      async openReleasePage() {
        await call<null>("update_open_release_page", {});
      },
    },
  };
}

/** What `terminal_attach` answers (`vw_host::TerminalAttached`). */
interface Attached {
  viewer: number;
  start: number;
  end: number;
}

/**
 * Attaches through `terminal_attach`: raw bytes arrive on a Channel as `ArrayBuffer`s (the replay
 * may arrive before the command resolves, so acknowledgements wait for the viewer id).
 */
async function attachTauriTerminal(
  terminalId: string,
  onChunk: TerminalChunkHandler,
): Promise<TerminalStream> {
  let viewer: number | null = null;
  let pending = 0;
  let open = true;
  const ack = (bytes: number): void => {
    if (!open) return;
    if (viewer === null) {
      pending += bytes;
      return;
    }
    void call<null>("terminal_ack", { terminalId, viewer, bytes }).catch(() => undefined);
  };
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buffer) => {
    if (!open) return;
    const chunk = new Uint8Array(buffer);
    onChunk(chunk, () => {
      ack(chunk.byteLength);
    });
  };
  const attached = await call<Attached>("terminal_attach", { terminalId, output: channel });
  viewer = attached.viewer;
  if (pending > 0) {
    const bytes = pending;
    pending = 0;
    ack(bytes);
  }
  return {
    detach() {
      if (!open) return;
      open = false;
      channel.onmessage = () => undefined;
      void call<null>("terminal_detach", { terminalId, viewer: attached.viewer }).catch(
        () => undefined,
      );
    },
  };
}
