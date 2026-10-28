/**
 * The "New copy" part of the new-chat view as one hook: the folder's
 * git facts from the core (`worktrees options`), the toggle with its default, the base branch, the first
 * message (the new-chat composer's text, set by the view) and the branch name suggested from it (`suggestName`,
 * debounced, until the user edits the name). The copy starts without a first turn: the composer sends it.
 * Depends on: state/worktrees (`run`, `start`), model.ts. Called by: sessions/NewChatPanel.
 * Invariants: answers are kept with the folder they are for (a changed folder never shows the old one); a
 * failed read hides the section (starting in the folder itself stays possible); the toggle's last value is
 * remembered; every request is the window's (a folder a Debug API client chose was picked for this run).
 */
import { useEffect, useState } from "react";

import type { IncludeReport, WorktreeOptions } from "../../protocol/core";
import type { NewFolderBy } from "../../state/shellState";
import { NO_WORKTREES, type Worktrees } from "../../state/worktrees";
import { useStore } from "../../state/createStore";
import { copyByDefault, rememberChoice, rememberedChoice } from "./model";

/** Debounce of the name suggestion while typing (ms). */
const SUGGEST_DELAY = 250;

/** The form's state and actions. */
export interface CopyForm {
  /** The folder is a git repository (the section shows). */
  available: boolean;
  /** Branch of the existing Vibe Work copy chosen as the chat folder. */
  existingBranch: string | null;
  /** The core did not read the folder: it was not chosen with Choose folder (`reason` says so). */
  notPicked: boolean;
  /** A copy may be made (else `reason`). */
  offered: boolean;
  reason: string | null;
  on: boolean;
  setOn(on: boolean): void;
  branches: readonly { name: string; current: boolean }[];
  /** The detached commit, when the checkout is detached (a base choice of its own). */
  detached: string | null;
  base: string;
  setBase(base: string): void;
  message: string;
  /** A stable setter (the new-chat view passes the composer's text as it changes). */
  setMessage: (text: string) => void;
  branch: string;
  setBranch(name: string): void;
  /** What `.worktreeinclude` would copy into the new copy; `null` when nothing is known. */
  include: IncludeReport | null;
  /** The start button starts a copy. */
  active: boolean;
  /**
   * Starts the session in a copy (no first turn: the new-chat composer sends it). A folder a Debug API client chose
   * was picked for this run when it chose it, so the window's start may trust it.
   */
  start(): Promise<string | null>;
}

function defaultBase(options: WorktreeOptions): string {
  const head = options.head;
  if (head?.kind === "branch") return head.name;
  if (head?.kind === "detached") return head.commit;
  return options.branches[0]?.name ?? "";
}

/**
 * The form for `folder`, chosen `by` the picker or a Debug API client (a native pick refreshes the options even when the folder spelling stays the same).
 */
export function useCopyForm(
  folder: string | null,
  by: NewFolderBy | null,
  othersLive: boolean,
  worktrees: Worktrees | null,
): CopyForm {
  const [found, setFound] = useState<{ folder: string; options: WorktreeOptions } | null>(null);
  // The user's choice in this panel (`null`: none yet, the default applies).
  const [choice, setChoice] = useState<{
    folder: string | null;
    request: number;
    on: boolean;
  } | null>(null);
  const newChatRequest = useStore(worktrees?.store ?? NO_WORKTREES, (s) => s.newChatRequest);
  const requestedCopy = useStore(worktrees?.store ?? NO_WORKTREES, (s) => s.newChatCopy);
  const options = found !== null && found.folder === folder ? found.options : null;
  const existingBranch = options?.existingBranch ?? null;
  const on =
    choice?.folder === folder && choice.request === newChatRequest
      ? choice.on
      : (requestedCopy ?? copyByDefault(othersLive, rememberedChoice(), existingBranch !== null));
  const [base, setBase] = useState("");
  const [message, setMessage] = useState("");
  const [branch, setBranchState] = useState("");
  const [edited, setEdited] = useState(false);

  useEffect(() => {
    if (folder === null || worktrees === null) return;
    let live = true;
    worktrees
      .run({ op: "options", cwd: folder })
      .then((result) => {
        if (!live || result.kind !== "options") return;
        setFound({ folder, options: result.options });
        setChoice(null);
        setBase(defaultBase(result.options));
        setBranchState(result.options.suggestion ?? "");
        setEdited(false);
      })
      .catch(() => {
        if (live) setFound(null);
      });
    return () => {
      live = false;
    };
  }, [folder, by, worktrees, newChatRequest]);

  // the message is the new-chat composer's text, so it changes with every key. A name is suggested only
  // while a copy would be made (a git folder, offered, the switch on): a plain folder is never asked (the core
  // refuses a non-git folder, which showed as a refusal toast on macOS where the test folder is no repository).
  const copyOn =
    found !== null && found.folder === folder && found.options.isGit && found.options.offered && on;
  useEffect(() => {
    if (!copyOn || edited || worktrees === null) return;
    let live = true;
    const timer = setTimeout(() => {
      worktrees
        .run({ op: "suggestName", cwd: folder, text: message })
        .then((result) => {
          if (live && result.kind === "name") setBranchState(result.branch);
        })
        .catch(() => undefined);
    }, SUGGEST_DELAY);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [copyOn, message, edited, folder, worktrees]);

  const available = options?.isGit === true;
  const offered = options?.offered === true;
  const head = options?.head;
  return {
    available,
    existingBranch,
    notPicked: options?.picked === false,
    offered,
    reason: options?.reason ?? null,
    on,
    setOn(next) {
      setChoice({ folder, request: newChatRequest, on: next });
      rememberChoice(next);
    },
    branches: options?.branches ?? [],
    detached: head?.kind === "detached" ? head.commit : null,
    base,
    setBase,
    message,
    setMessage,
    branch,
    setBranch(name) {
      setBranchState(name);
      setEdited(true);
    },
    include: options?.include ?? null,
    active: available && offered && on && folder !== null,
    async start() {
      if (worktrees === null || folder === null) return null;
      return worktrees.start({ cwd: folder, base, branch: branch.trim(), firstMessage: "" });
    },
  };
}
