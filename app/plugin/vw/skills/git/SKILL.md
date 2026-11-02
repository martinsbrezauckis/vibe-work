---
name: git
description: Git in Vibe Work - the user's Commit, Push, Create PR and Bring into main buttons; which git commands the agent's tools run, ask about or refuse (pushes, branch rewrites, credentials, the Git identity, "Not run" answers); GitHub reads; pull request check and review messages.
---

In Vibe Work the user sends work out with buttons in the Changes tab (also in the Git tab): **Commit**, **Push**, **Create PR** and, for a session copy, **Bring into main**. Push never force-pushes.

1. **Run with Vibe's access-mode card:** read-only git (`git status`, `git diff`, `git log`, `git show`, `git blame`), `git add`, and local commits, merges, cherry-picks, reverts, am and pulls in this folder or the session's own copy. Open files may hold unsaved edits (`vw:editor`).
2. **Refused: sending to a remote.** A `git push` answers exactly:
   `Not run: sending commits to a remote is the user's step in Vibe Work. No command in this line ran. Existing commits are unchanged. The Push button is in the Changes tab.`
   A folder link opens the Changes tab on that repository: `[Changes](git%20test/)`, or `[Changes](./)` for the session repository.
3. **Asks the user first, in every mode** (a Vibe Work dialog): common branch rewrites outside the session's own copy branch, such as `git rebase`, `git reset` to another commit, `git update-ref`, `git branch -f|-d|-D|-m`, `git checkout -B`, `git switch -C`, or a fetch/pull refspec or worktree option that writes another branch; and, even in the own copy, `git stash drop`/`clear` (every copy shares the stashes) and `git worktree remove`/`move`. A preceding folder or branch switch keeps a later rewrite held. Ordinary local operations and config use Vibe's access-mode card.
   The Git identity is the user's: setting it (`git config user.*`, `author.*`, `committer.*`, `git -c user.*=`, `--author`, `GIT_AUTHOR_*`, `GIT_COMMITTER_*`) is refused. When Git has no name or email, `git init` and `git commit` wait while Vibe Work asks the user once; Cancel answers `Not run: No Git identity.`
4. **GitHub reads:** `gh --version` and `gh help` run without credentials. Settings offers optional read-only access for supported `gh` commands. Without gh, the user can install GitHub CLI and restart Vibe Work, or the GitHub connector reads. Writes are the user's Push / Create PR buttons.
5. **No Git or forge sign-in in the agent's commands.** Credential helpers and Git prompts are disabled; token and SSH-agent variables are removed. This applies to Vibe's tools, process tools, secret commands and integrated terminals, so private fetches can fail; the user's own Git tool has the sign-in. The core's Push and Create PR buttons use the user's sign-in. `git credential ...`, `gh auth token` and `gh auth status --show-token` answer exactly: `Not run: the agent cannot use the user's stored Git or GitHub sign-in. Continue without reading credentials; the user can push or create a PR in the Changes tab.`
6. **Messages from the PR card.** "These checks failed on pull request #N" and "Review comments on pull request #N" come from the user's buttons. "Send failing checks" quotes the failing lines; the GitHub connector reads the whole run. "Bring the latest base into this branch" is the message of Ask Vibe to update the copy from its base, which the user sends.
7. Pushing files, creating or merging pull requests, branches, issues, comments or repositories through a connector (GitHub tools) is the same step as `git push`: such calls answer `Not run:`. Reading through the connector works.
8. `git commit -F <file>` with a long message shows the user a short command to confirm.
9. A new copy has no installed dependencies; its setup hint names the install command. A file whose header says GENERATED or "do not edit" comes from a generator.
10. Agent Bash commands run with `pipefail` on Linux; on macOS and Windows it is not guaranteed, so `check | tail` can report `tail`'s exit code there. With `pipefail`, a `| head` that stops reading early fails the command (exit 141).

The command guards check literal command starts after assignments and a few transparent wrappers. They do not inspect scripts, aliases, quotes or computed commands: they are a reminder against accidental actions, not a sandbox.
