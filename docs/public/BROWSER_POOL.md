# Browser tabs and memory allowance

The Browser panel has a pinned **App preview** for the session's own local servers and
separate web tabs. Switching pages preserves each tab's address and native view. The agent
uses its current web tab; its browsing never replaces App preview. User-created web tabs
require a reviewed handoff before the agent can control them. Taking a tab back cancels
pending agent use. Tab handoff grants no vault access.

These limits apply to Vibe Work's Browser tab. Your Chrome uses Vibe Work Browser Control separately; see
[setup and permissions](BROWSER_CONTROL.md).

Settings > Browser controls how many Browser page engines can stay live across sessions.
**Auto** is the default. Fixed choices are 1, 2, 3 and 4; memory can reduce any choice.
The Settings line shows detected total and available memory and the current effective cap.

The total-memory allowance is 1 engine below 8 GiB, 2 from 8 GiB, 3 from 16 GiB and 4 from
28 GiB. Available memory reduces this to 1 below 1,536 MiB or 2 below 3,072 MiB. The effective
cap is the smallest of these allowances and a fixed setting, with a minimum of 1. Missing memory
information allows 1. On Linux, readable unified cgroup limits also constrain the allowance.

The pool includes the pinned App preview and every web tab, including pages of the session's
own local servers. The user-only Settings sign-in page retains its separate behavior. Admission is checked at the
engine mount boundary, including reads and layouts that restore a suspended page.

When a tab needs an engine and the pool is full, it closes the least recently used
eligible background engine first. The tab retains its address, website data and last safe
visible snapshot. Selecting a suspended tab reloads its page. Snapshots are omitted after a
vault login fill, and pages that were never visible have no snapshot.
The pool retains the visible page, the agent's current page during an unfinished turn
(including one waiting for the user's answer), ongoing page operations and a pending user save.
If every slot is protected,
the browser tool immediately answers:

> Not run: the browser is busy with another session; try again when it is free

The requested address is retained, and the panel displays **Waiting for the browser**. There is
no automatic queue execution: the agent retries explicitly when a slot becomes available.
A denied session stays waiting when its turn ends; it has no engine to release.

The selected browser panel remains protected while a modal temporarily parks its native view.
Closing the panel, switching pages or switching to another session removes that page's panel
protection; the agent's current page and pending saves retain their own protection.

Turn end keeps the document alive. Hiding the panel, switching sessions, or opening a modal only
parks the view. A name prompt keeps its one-use save choice until the user finishes or cancels,
and the save checks the same document before confirmation. It has no fixed two-minute expiry.
The periodic idle check may close an eligible hidden engine after ten minutes without use;
hiding, turn completion, save completion and page operations renew that grace period. A visible
panel, unfinished turn or pending save remains protected regardless of age. Lowering the setting
or falling available memory changes new-engine admission; existing documents are not proactively
closed to meet a lower cap. Eviction closes the engine before freeing its slot and retains its URL
and website data. A failed engine close retains its admission rather than permitting another
engine to consume the same slot.
Session deletion and handoff also retain a mount whose close failed, so a later close can retry
with the original identity instead of losing track of the engine.

The typed command API also exposes the pool to the Debug API with user rights:

```json
{"type":"browser","request":{"op":"enginePool"}}
```

```json
{"type":"browser","request":{"op":"setEnginePool","configuredParallelSessions":"1"}}
```

Both return a browser result of type `enginePool`, containing the saved setting, effective cap,
detected memory, active and waiting page IDs, and the idle timeout. The existing wire field names
`activeSessions` and `waitingSessions` now contain page keys. The preference is persisted
in `engine-pool.json` under the preview data root; a fresh profile starts at Auto.
