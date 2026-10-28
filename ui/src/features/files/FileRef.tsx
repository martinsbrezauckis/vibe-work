/**
 * File references in the transcript: a path the chat shows becomes a button that opens the File tab,
 * but only after the core said it names a file the app may show (`files resolve`); until then, and when the core
 * says no, it stays exactly the text it was. Provided per transcript (`FileRefsProvider`), so a reference always
 * resolves against the session it belongs to.
 *
 * Depends on: AppContext (`files`, `shell`), state/files, remarkFileRefs, model/refs. Called by: the transcript
 * (provider, Markdown extension, tool output and file steps), tests.
 * Invariants: only a resolved file or explicit choices are clickable for this session; the visible text never
 * changes. A click opens the File tab or chooser; FileRefLink also offers the shared file actions.
 */
import {
  type ReactElement,
  type ReactNode,
  createContext,
  useContext,
  useEffect,
  useState,
} from "react";

import type { MarkdownExtension } from "../../components/markdown/Markdown";
import { createStore, useStore } from "../../state/createStore";
import type { Files, FilesState } from "../../state/files";
import type { Git } from "../../state/git";
import { useApp } from "../shell/AppContext";
import { findPathRefs } from "./model/refs";
import { FILE_REF_TAG, remarkFileRefs } from "./remarkFileRefs";
import { FileRefLink } from "./FileRefLink";

interface Refs {
  sessionId: string;
  files: Files;
  open: (file: string, display: string, line: number | null) => void;
}

const Context = createContext<Refs | null>(null);

/** The transcript session for navigation from its completed edit steps. */
export function useFileSessionId(): string | null {
  return useContext(Context)?.sessionId ?? null;
}

/** A store that never changes, for references rendered without a provider (hooks need a store either way). */
const NO_STORE = createStore<FilesState>({ refs: {}, tabs: {}, drops: {}, edits: {} });

/** Makes file references inside `children` resolve against `sessionId` (no-op without a core). */
export function FileRefsProvider({
  sessionId,
  children,
}: {
  sessionId: string;
  children: ReactNode;
}): ReactElement {
  const { files, shell, toasts } = useApp();
  if (files === null) return <>{children}</>;
  const open = (file: string, display: string, line: number | null) => {
    shell.actions.openTab("file");
    void files.actions.show(sessionId, { file, display, line }).catch((error: unknown) => {
      toasts.push({
        kind: "warn",
        bellKind: "failure",
        text: error instanceof Error ? error.message : "Could not open the file.",
      });
    });
  };
  return <Context.Provider value={{ sessionId, files, open }}>{children}</Context.Provider>;
}

/** A folder reference is a Changes link only when the core discovered that exact repository root. */
function FolderRef({ path, children }: { path: string; children: ReactNode }): ReactElement {
  const refs = useContext(Context);
  const { git } = useApp();
  if (refs === null || git === null) return <span data-repo-ref={path}>{children}</span>;
  return (
    <KnownFolderRef git={git} sessionId={refs.sessionId} path={path}>
      {children}
    </KnownFolderRef>
  );
}

function KnownFolderRef({
  git,
  sessionId,
  path,
  children,
}: {
  git: Git;
  sessionId: string;
  path: string;
  children: ReactNode;
}): ReactElement {
  const { shell } = useApp();
  const knownRoot = useStore(git.store, (s) => {
    const status = s.status[sessionId];
    const exact = status?.repos.find((repo) => repo.root === path);
    if (exact !== undefined) return exact.root;
    const root = status?.state?.root;
    const windows =
      root !== undefined && /^(?:[a-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+|\/\/[^/]+\/[^/]+)/i.test(root);
    return windows
      ? (status?.repos.find((repo) => repo.root.toLowerCase() === path.toLowerCase())?.root ?? null)
      : null;
  });
  if (knownRoot === null) return <span data-repo-ref={path}>{children}</span>;
  return (
    <button
      data-testid="files.file-ref-select-repo"
      type="button"
      className="fref"
      data-repo-ref={path}
      aria-label={`Open ${path} in Changes tab`}
      title={`Open ${path} in Changes tab`}
      onClick={() => {
        void git.selectRepo(sessionId, knownRoot);
        shell.actions.openTab("changes");
      }}
    >
      {children}
    </button>
  );
}

/** One reference: `children` as written, clickable after the core verified a file or repository. */
export function FileRef(props: {
  path: string;
  line: number | null;
  folder?: boolean;
  children: ReactNode;
}): ReactElement {
  return props.folder ? (
    <FolderRef path={props.path}>{props.children}</FolderRef>
  ) : (
    <ResolvedFileRef {...props} />
  );
}

/** A file reference opens the File tab only after file resolution succeeds. */
function ResolvedFileRef({
  path,
  line,
  children,
}: {
  path: string;
  line: number | null;
  children: ReactNode;
}): ReactElement {
  const refs = useContext(Context);
  const state = useStore(refs?.files.store ?? NO_STORE, (s) =>
    refs === null ? undefined : s.refs[refs.sessionId]?.[path],
  );
  const missing = state === undefined;
  useEffect(() => {
    if (refs !== null && missing) refs.files.want(refs.sessionId, path);
  }, [refs, missing, path]);
  if (refs === null) return <>{children}</>;
  if (state?.status !== "ok" && state?.status !== "choices")
    // Marked (for the `ui/files refs` route) but plain: nothing about a refused path is shown.
    return (
      <span className="fref-text" data-file-ref={path} data-file-state={state?.status ?? "pending"}>
        {children}
      </span>
    );
  return (
    <FileRefLink sessionId={refs.sessionId} path={path} line={line} state={state}>
      {children}
    </FileRefLink>
  );
}

/** Paths that look like a web address: never resolved as a file (an image from the web stays text). */
const WEB = /^[a-z][a-z0-9+.-]*:/i;

/**
 * an image the agent saved in the session folder and showed with `![alt](images/x.png)`. It shows only after
 * the core resolved the path inside this session's scope as an image and read it (a `data:` URL); until then, and for
 * anything else (a web address, a missing file, not an image), it is the alt text, exactly as before.
 */
function InlineImage({ src, alt }: { src: string; alt: string }): ReactElement {
  const refs = useContext(Context);
  const local = !WEB.test(src) ? src : null;
  const state = useStore(refs?.files.store ?? NO_STORE, (s) =>
    refs === null || local === null ? undefined : s.refs[refs.sessionId]?.[local],
  );
  const missing = state === undefined;
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (refs !== null && local !== null && missing) refs.files.want(refs.sessionId, local);
  }, [refs, local, missing]);
  const handle = state?.status === "ok" && state.kind === "image" ? state.file : null;
  useEffect(() => {
    let live = true;
    if (refs !== null && handle !== null)
      void refs.files.image(refs.sessionId, handle).then((u) => {
        if (live) setUrl(u);
      });
    return () => {
      live = false;
    };
  }, [refs, handle]);
  if (refs === null || handle === null || url === null || !url.startsWith("data:image/"))
    return <span className="md-image-blocked">{alt}</span>;
  return (
    <button
      data-testid="files.file-ref-open-2"
      type="button"
      className="md-image"
      title={`Open ${state?.status === "ok" ? state.display : alt} in the File tab`}
      aria-label={`Open ${state?.status === "ok" ? state.display : alt} in the File tab`}
      data-file-ref={local ?? ""}
      onClick={() => {
        if (state?.status === "ok") refs.open(state.file, state.display, null);
      }}
    >
      <img src={url} alt={alt} />
    </button>
  );
}

/** The Markdown extension that marks references (`Markdown extension={FILE_REFS}`). */
export const FILE_REFS: MarkdownExtension = {
  plugin: remarkFileRefs as MarkdownExtension["plugin"],
  tag: FILE_REF_TAG,
  render: (properties, children) => {
    const path = typeof properties["dataPath"] === "string" ? properties["dataPath"] : "";
    const folder = properties["dataFolder"] === "true";
    const raw = properties["dataLine"];
    const line = typeof raw === "string" && /^\d+$/.test(raw) ? Number(raw) : null;
    return path === "" ? (
      <>{children}</>
    ) : (
      <FileRef path={path} line={line} folder={folder}>
        {children}
      </FileRef>
    );
  },
  image: (src, alt) => <InlineImage src={src} alt={alt} />,
};

/** Plain text (tool output) with its file references clickable; at most `max` references. */
export function LinkedText({ text, max = 50 }: { text: string; max?: number }): ReactElement {
  const refs = findPathRefs(text, max);
  if (refs.length === 0) return <>{text}</>;
  const parts: ReactNode[] = [];
  let at = 0;
  for (const [i, ref] of refs.entries()) {
    if (ref.start > at) parts.push(text.slice(at, ref.start));
    parts.push(
      <FileRef key={i} path={ref.path} line={ref.line}>
        {text.slice(ref.start, ref.end)}
      </FileRef>,
    );
    at = ref.end;
  }
  if (at < text.length) parts.push(text.slice(at));
  return <>{parts}</>;
}
