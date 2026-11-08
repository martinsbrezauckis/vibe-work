"""Restore an explicitly unarchived copy using pinned Vibe's Git-only recovery.

Called by worktrees_restore with the saved cwd and app-owned repository record.
Uses Vibe's claim validation and prune lock; never creates a session or runtime.
"""
import json
import os
import sys
from pathlib import Path

from vibe.core.git.worktree.repository import ManagedWorktree


def same_folder(first: Path, second: Path) -> bool:
    """One folder however it is spelled (Windows verbatim ``\\\\?\\`` prefix, case, ``/``); missing is never the same.

    Vibe keeps the repository root as the chat's start folder was spelled, the app record keeps Git's spelling, and
    ``Path.resolve`` keeps a verbatim prefix on Windows, so equal paths are not required, only the same folder.
    """
    try:
        return os.path.samefile(first, second)
    except OSError:
        return False


def main() -> None:
    cwd, repository = (Path(value) for value in sys.argv[1:])
    worktree = ManagedWorktree.at(cwd)
    if worktree is None:
        raise ValueError("The saved copy has no managed worktree claim")
    recovery = worktree.claim.read_recovery()
    if recovery is not None and not same_folder(recovery.repo_root, repository):
        raise ValueError("The saved copy belongs to a different repository")
    print(json.dumps({"restored": worktree.restore(cwd)}))


if __name__ == "__main__":
    main()
