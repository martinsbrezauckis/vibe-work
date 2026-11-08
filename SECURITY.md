# Security

## Reporting a problem

Please report security problems privately, not in a public issue:

- through GitHub's private vulnerability reporting on this repository (Security › Report a vulnerability), or
- by email to martins.brezauckis@gmail.com.

Say what you found, how to reproduce it, and which version and system you used. You will get an answer within a
few days. Fixes are released as soon as they are ready, and the release notes credit you unless you ask otherwise.

Problems in Vibe itself (the `mistral-vibe` package that Vibe Work runs) belong to Mistral AI: report them to
Mistral. If you are not sure which side a problem is on, report it here and it will be passed on.

## What is in scope

Vibe Work's own code in this repository: the Rust core and host, the Tauri app, the user interface, the Debug API,
the local key proxy and vault handling, the installers and the update flow.

## How Vibe Work is built

- **The main window renders no untrusted active content.** Model output is shown as sanitized Markdown without raw
  HTML, under a strict content security policy. Web pages, app previews and handbook pages open in separate,
  isolated webviews that cannot call the app.
- **Every request to Vibe goes through one method gate.** Unknown methods are refused; requests that install things,
  add servers or send data elsewhere need your confirmation in a native dialog, and a folder can be trusted only
  after you chose it yourself.
- **Vibe never receives your Mistral key.** It is kept in an encrypted vault. Vibe is given a placeholder, and the real
  key is added to Mistral requests by a local proxy that only listens on this computer.
- **Local network proxies require their own credentials.** Each built-in browser store and each app, Vibe or
  connection-test forwarder has a separate random credential for its listener's lifetime. Requests without it get
  a proxy authentication challenge before any destination lookup or upstream connection. Closing a listener ends
  its connections; a replacement gets a new credential. Native browser engines receive it privately. Vibe receives
  only its local credential, in its user-only configuration and environment; saved upstream proxy sign-ins stay
  in the vault.
- **The Debug API is off in release builds** until you turn it on in Settings. When on, it listens on 127.0.0.1
  only and needs the token stored in your vault, generated and copied in Settings while the vault is unlocked.
  Release builds do not export it to a plaintext token file. It can then do what you can do in the app, except on
  the vault, so turn it on only for programs you trust. On the vault it has the agent's rights only: managing the
  vault, the Mistral key, adding or removing secrets and changing permissions (revoking a grant) stay yours, in the
  window, and the dialogs that let a secret be stored or used are answered only by you. It hides known secret values
  in its answers where it can; that is a convenience, not a guarantee.
- **Browser access includes local addresses.** Your Chrome and Your Edge use the normal per-site permission for local development servers, private networks and router pages. Access modes, tab ownership, password protection and Stop still apply. The extension now in the Chrome Web Store still refuses other programs' local ports and link-local addresses until its next version. Your Safari uses Safari's website permissions. The built-in browser keeps its engine's network behavior. Backend media fetching retains separate private-address checks.
- **Processes are tracked by owner.** Session-started processes are listed with their session and ports. Dev servers
  the agent started keep running until you stop them; deleting a session asks whether to stop them.
- **Updates are signed.** The app installs an update only after its signature checks out and you agreed.

## Known limits

- The agent runs commands with your user's permissions. Vibe's access modes and Vibe Work's Plan on a page profile
  decide what may run without asking; see [Access modes](docs/public/VOCABULARY.md#access-modes) for the current names
  and behavior. Vibe Work adds no operating-system sandbox in this version. The ordinary agent command environment
  strips inherited Git and forge sign-in channels; the command guard refuses push, GitHub writes and Git/gh
  credential commands. Common branch rewrites outside the session's own worktree branch need a separate
  confirmation. These guardrails do not isolate other same-user credentials or interpret every shell expression: a No
  in a Vibe Work prompt refuses that call, the turn goes on, and the same action written another way (an
  interpreter one-liner, for example) is not held.
- Pairing Your Chrome or Your Edge asks you first in a Vibe Work dialog; pairings made before 0.1.8 keep working. A
  program running as your user that can both register a Chrome native host and open Chrome is outside this model.
- The core process holds the Mistral key in memory. On Linux core dumps are off and other processes of the same user
  cannot attach a debugger to it or read its memory; on macOS only core dumps are turned off, and on Windows other
  processes of the same user can read its memory.
- Browser web tabs are separate from the pinned App preview. The agent lists only its session's web tabs. In Ask,
  the built-in browser refuses to close a tab you opened; Your Chrome and Your Edge ask before closing a tab. In
  Accept edits and Full access, closing a tab you opened runs without a card. The agent cannot close the pinned
  App preview or another session's tab.
- On Linux, the built-in Browser's reads leave out a frame that cannot run scripts inside a closed shadow root without
  reporting it as unreadable; a click still checks its own target.
