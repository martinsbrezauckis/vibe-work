/** Commit story and lazy version details. Uses shared list controls; workspace actions and opening come from FilesPanel. */
import { useEffect, useRef, type ReactNode } from "react";
import { Button, LinkButton, RowDisclosure } from "../../components/Controls";
import { EmptyState } from "../../components/EmptyState";
import { Icon } from "../../components/Icon";
import { PathText } from "../../components/PathText";
import { RowMenuButton, type RowMenuState } from "../../components/RowMenu";
import type { ExplorerTarget } from "./explorerMenus";
import type { ExplorerVersion } from "../../protocol/core";
import type { Explorer, ExplorerState } from "./store";

/** Counts with thousands separators ("80,000"), as the rest of the app writes them. */
export const n = (value: number): string => value.toLocaleString("en-US");

/** Local-day grouping follows the person's clock, including around midnight. */
export function versionDay(time: number, now = new Date()): string {
  const date = new Date(time * 1000),
    today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (date >= today) return "Today";
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (date >= yesterday) return "Yesterday";
  return date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}
/** A row's ⋯ for the tab's one menu. */
function More({
  menu,
  target,
  id,
  words,
}: {
  menu: RowMenuState<ExplorerTarget> | null;
  target: ExplorerTarget;
  id: string;
  words: string;
}) {
  if (menu === null) return null;
  return (
    <RowMenuButton
      testId="explorer.history-more-actions"
      itemId={id}
      label={`Actions for ${words}`}
      expanded={menu.at !== null && JSON.stringify(menu.at.target) === JSON.stringify(target)}
      onOpen={(button) => {
        menu.openBelow(button, target);
      }}
    />
  );
}
export function VersionRow({
  version,
  onOpen,
  menu = null,
}: {
  version: ExplorerVersion;
  onOpen: () => void;
  menu?: RowMenuState<ExplorerTarget> | null;
}) {
  const target: ExplorerTarget = { kind: "version", version };
  return (
    <div className="fx-version-row" {...menu?.triggers(target)}>
      <Button data-testid="explorer.open-version" data-item-id={version.id} onClick={onOpen}>
        <span className="fx-version-subject" title={version.subject}>
          {version.subject}
        </span>
        <span className="ui-caption">
          {version.who} ·{" "}
          {new Date(version.time * 1000).toLocaleTimeString(undefined, {
            hour: "2-digit",
            minute: "2-digit",
          })}{" "}
          ·{" "}
          {version.published === true
            ? "in the shared history"
            : version.published === false
              ? "only on this computer"
              : "where it is shared is not known"}
        </span>
      </Button>
      <More menu={menu} target={target} id={version.id} words={`the commit ${version.subject}`} />
    </div>
  );
}
export function History({
  state,
  explorer,
  session,
  onOpen,
  onReview,
  workspaceActions,
  places,
  onInit,
  menu = null,
  onCompare,
}: {
  state: ExplorerState;
  explorer: Explorer;
  session: boolean;
  onOpen: (path: string) => void;
  /** The tab's one menu (right click, menu key, ⋯); null: no menu. */
  menu?: RowMenuState<ExplorerTarget> | null;
  /** A file inside a commit: compare it with that commit's version. */
  onCompare: (path: string, version: ExplorerVersion) => void;
  onReview: () => void;
  workspaceActions: ReactNode;
  places: ReactNode;
  onInit: () => void;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  const lastVersion = useRef<string | null>(null);
  useEffect(() => {
    if (state.version) {
      lastVersion.current = state.version.id;
      return;
    }
    if (scroll.current) {
      scroll.current.scrollTop = state.historyScroll;
      if (lastVersion.current) {
        scroll.current
          .querySelector<HTMLButtonElement>(`button[data-item-id="${lastVersion.current}"]`)
          ?.focus({ preventScroll: true });
        lastVersion.current = null;
      }
    }
  }, [state.version, state.historyScroll]);
  const changed = (state.status?.changedFiles ?? 0) + (state.status?.untrackedFiles ?? 0);
  if (state.version)
    return (
      <div
        data-testid="explorer.version-detail"
        className="fx-scroll"
        onKeyDown={(event) => {
          if (event.key === "Escape" || event.key === "Backspace") {
            event.preventDefault();
            explorer.back();
          }
        }}
      >
        <LinkButton
          data-testid="explorer.history-back"
          onClick={() => {
            explorer.back();
          }}
        >
          ← History
        </LinkButton>
        <h3 className="ui-heading">{state.version.subject}</h3>
        <p className="ui-help">
          Committed {new Date(state.version.time * 1000).toLocaleString()} by {state.version.who}.
        </p>
        <p className="key-line">
          {state.versionLoading
            ? "Reading changed files…"
            : `${n(state.versionFiles.length)} files changed.`}
        </p>
        {state.versionFiles.map((path) => {
          const version = state.version;
          if (version === null) return null;
          const target: ExplorerTarget = { kind: "versionFile", version, path };
          return (
            <div key={path} className="fx-file-row" {...menu?.triggers(target)}>
              <LinkButton
                data-testid="explorer.version-file"
                data-item-id={path}
                onClick={() => {
                  onCompare(path, version);
                }}
              >
                <PathText path={path} />
              </LinkButton>
              <More menu={menu} target={target} id={path} words={path} />
            </div>
          );
        })}
        {state.versionLimited && <p className="key-notice">Showing the first 5,000 files.</p>}
      </div>
    );
  if (state.historyNote)
    return (
      <div className="fx-scroll">
        <p className="key-problem" role="alert">
          {state.historyNote}
        </p>
        {places}
      </div>
    );
  if (state.statusLoading && !state.historyLoaded)
    return (
      <p className="key-line" role="status">
        Reading version history…
      </p>
    );
  if (state.status === null && !state.loadingFolders.includes(""))
    return (
      <div className="fx-scroll">
        <EmptyState glyph="folder" small>
          <p>This folder has no version history yet.</p>
          <Button data-testid="explorer.enable-history" shape="outlined" onClick={onInit}>
            Turn on version history…
          </Button>
        </EmptyState>
        {places}
      </div>
    );
  return (
    <div
      className="fx-scroll"
      ref={scroll}
      data-testid="explorer.history-list"
      onKeyDown={(event) => {
        if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
        const buttons = [
          ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
            '[data-testid="explorer.open-version"], [data-testid="explorer.pending"]',
          ),
        ];
        const at = buttons.indexOf(event.target as HTMLButtonElement);
        if (at < 0) return;
        event.preventDefault();
        const next =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? buttons.length - 1
              : Math.max(
                  0,
                  Math.min(buttons.length - 1, at + (event.key === "ArrowDown" ? 1 : -1)),
                );
        buttons[next]?.focus();
      }}
      onScroll={(e) => {
        explorer.scroll(e.currentTarget.scrollTop);
      }}
    >
      {state.historyPath !== null && (
        <p className="ui-help fx-info" data-testid="explorer.history-of">
          <Icon name="file" size={14} />
          <span>History of {state.historyPath}.</span>
          <LinkButton
            data-testid="explorer.history-all"
            onClick={() => {
              explorer.historyOf(null);
            }}
          >
            Show all commits
          </LinkButton>
        </p>
      )}
      <div className="fx-pending" {...menu?.triggers({ kind: "pending" })}>
        <div className="fx-pending-head">
          <RowDisclosure
            data-testid="explorer.pending"
            aria-expanded={state.pendingOpen}
            onClick={() => {
              explorer.setPending(!state.pendingOpen);
            }}
          >
            <span>Changes since the last commit</span>
            <span className="ui-caption">
              {n(changed)} {changed === 1 ? "file" : "files"} · not committed yet
            </span>
          </RowDisclosure>
          <More
            menu={menu}
            target={{ kind: "pending" }}
            id="pending"
            words="the changes since the last commit"
          />
        </div>
        {state.pendingOpen && (
          <>
            {state.status?.files.map((file) => (
              <div
                className="fx-file-row"
                key={file.path}
                {...menu?.triggers({ kind: "file", path: file.path, place: "files" })}
              >
                <LinkButton
                  data-testid="explorer.changed-file"
                  data-item-id={file.path}
                  onClick={
                    session
                      ? onReview
                      : () => {
                          onOpen(file.path);
                        }
                  }
                >
                  <PathText path={file.path} />
                </LinkButton>
                <More
                  menu={menu}
                  target={{ kind: "file", path: file.path, place: "files" }}
                  id={file.path}
                  words={file.path}
                />
              </div>
            ))}
            {session ? (
              <LinkButton data-testid="explorer.review-session" onClick={onReview}>
                Open the Changes tab
              </LinkButton>
            ) : (
              workspaceActions
            )}
          </>
        )}
      </div>
      {state.versions.map((version, index) => {
        const next = versionDay(version.time);
        const previous = state.versions[index - 1];
        const header = previous === undefined || next !== versionDay(previous.time);
        return (
          <div key={version.id}>
            {header && <h3 className="ui-caption fx-day">{next}</h3>}
            <VersionRow
              version={version}
              onOpen={() => void explorer.openVersion(version)}
              menu={menu}
            />
          </div>
        );
      })}
      {state.historyLoading && (
        <p className="key-line" role="status">
          Reading commits…
        </p>
      )}
      {state.historyLoaded && state.versions.length === 0 && (
        <p className="key-line">No commits yet.</p>
      )}
      {state.cursor && (
        <Button
          data-testid="explorer.history-more"
          shape="outlined"
          disabled={state.historyLoading}
          onClick={() => void explorer.history(true)}
        >
          Show 50 more
        </Button>
      )}
      {places}
    </div>
  );
}
