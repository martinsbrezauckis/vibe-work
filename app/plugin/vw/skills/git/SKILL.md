---
name: git
description: Git in Vibe Work - Commit, app push tool, Create PR and Bring into main; which agent git commands run, ask or refuse; GitHub reads and pull request messages.
---

Vibe Work offers **Commit**, **Push**, **Create PR** and **Bring into main** in Changes. With Developer permissions on in Settings > Developer, the app's `push` tool uses the user's Git sign-in; shell `git push` stays refused. Push never force-pushes and sends only the confirmed branch: tags and submodule commits never go with it, whatever the Git config says; the user pushes tags with their own Git tool.

1. **Run with Vibe's access-mode card:** read-only git (`git status`, `git diff`, `git log`, `git show`, `git blame`), `git add`, and local commits, merges, cherry-picks, reverts, am and pulls in this folder or the chat's own copy. Open files may hold unsaved edits (`vw:editor`).
2. **Shell push stays refused.** When Developer permissions is off, a `git push` answers:
   `Git guard: Not run: sending commits to a remote is the user's step in Vibe Work. No command in this line ran. Existing commits are unchanged. For remote changes, ask the user to use Push or Create PR in the folder's Changes tab, or make the change on the code hosting site.`
   A folder link opens Changes: `[Changes](git%20test/)`, or `[Changes](./)` for the chat repository.
3. **Branch rewrites outside the chat's own copy branch** (`git rebase`, `reset`, `update-ref`, `branch -f`, `checkout -B`, `switch -C`) ask in every mode while Developer permissions is off. When on, they follow the chat's access mode like an ordinary local change. Shared-stash drop/clear, worktree remove/move and `git symbolic-ref` still ask. A preceding folder or branch switch keeps a later rewrite held while the switch is off.
   The Git identity is the user's: setting it (`git config user.*`, `author.*`, `committer.*`, `git -c user.*=`, `--author`, `GIT_AUTHOR_*`, `GIT_COMMITTER_*`) is refused. When Git has no name or email, `git init` and `git commit` wait while Vibe Work asks the user once; Cancel answers `Git guard: Not run: no Git identity: the user did not give a name and email. Continue without committing and tell the user.`
4. **GitHub reads:** `gh --version` and `gh help` run without credentials. Settings offers optional read-only access for supported `gh` commands. Without gh, the user can install GitHub CLI and restart Vibe Work, or the GitHub connector reads. Forge writes stay refused in the agent shell.
5. **No Git or forge sign-in in the agent's commands.** Credential helpers and Git prompts are disabled; token and SSH-agent variables are removed. This applies to Vibe's tools, process tools, secret commands and integrated terminals, so private fetches can fail; the user's own Git tool has the sign-in. `git credential ...`, `gh auth token` and `gh auth status --show-token` answer exactly: `Git guard: Not run: the agent cannot use the user's stored Git or GitHub sign-in. Continue without reading credentials; the user can push or create a PR in the Changes tab.`
6. **Messages from the PR card.** "These checks failed on pull request #N" and "Review comments on pull request #N" come from the user's buttons. "Send failing checks" quotes the failing lines; the GitHub connector reads the whole run. "Bring the latest base into this branch" is the message of Ask Vibe to update the copy from its base, which the user sends.
7. Pushing files, creating or merging pull requests, branches, issues, comments or repositories through a connector (GitHub tools) is the same step as `git push`: such calls answer `Not run:`. Reading through the connector works.
8. `git commit -F <file>` with a long message shows the user a short command to confirm.
9. A new copy has no installed dependencies; its setup hint names the install command. A file whose header says GENERATED or "do not edit" comes from a generator.
   Vibe's `enter_worktree` makes a chat copy that Vibe Work shows; ending the chat offers Keep or Throw away. The Git tab lists the chat's pull requests with check status.
10. Agent Bash commands run with `pipefail` on Linux; on macOS and Windows it is not guaranteed, so `check | tail` can report `tail`'s exit code there. With `pipefail`, a `| head` that stops reading early fails the command (exit 141).

The command guards check literal command starts after assignments and a few transparent wrappers. They do not inspect scripts, aliases, quotes or computed commands: they are a reminder against accidental actions, not a sandbox.
