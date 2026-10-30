# Use your Chrome with Vibe Work

**Not in Vibe Work 0.1.3.** This version has no Browser Control: Settings > Browser shows no Your Chrome section,
and Vibe works in Vibe Work's own browser. The rest of this page describes the setup for a version that includes it.

Vibe Work Browser Control lets Vibe use your own Chrome while you watch. It is separate from the Browser tab
inside Vibe Work, which continues to show App preview and its own web tabs.

## Set up

1. In Vibe Work, open Settings > Browser > Your Chrome and choose Set up….
2. Review the name, source and what will run, then choose Set up. This installs the helper that connects the app
   to Chrome and opens the Chrome Web Store page.
3. Add **Vibe Work Browser Control** in Chrome. Chrome shows its own prompt describing the extension's access.
4. Open the extension and pair it with Vibe Work. Compare the identity shown in Chrome with the identity in
   Vibe Work's setup window. The status changes to Paired when the connection is ready.
5. Choose the sites the extension may use in Chrome. Ask Vibe to use your Chrome; in that chat, answer
   Allow for this session when Vibe asks. If the extension asks you to approve a browser task, review it in Chrome.

Vibe Work links each Chrome task to the chat that requested it. A session answer lasts during the current app run; restarting
Vibe Work asks again. If the helper disconnects, Settings shows that it is not connected.

## Choose what Vibe may do

Settings > Browser > Your Chrome has three choices:

| Choice | What it allows without another action card |
| --- | --- |
| Basic browser control (default) | Read pages and see open tabs, go to sites, click, scroll and type into text fields, excluding password fields. |
| Advanced | Basic, plus fill forms without sending them, download files and upload a file you picked. |
| Full: works on your behalf | Advanced, plus send forms and confirm actions, use a vault login you allowed, change cookies and site data, and run page scripts. |

Actions above your choice ask in the chat. Bookmarks and browsing history always ask. The card describes what an
Allow once or Allow for this session answer covers. Chrome's site access still applies. Raising a saved level
asks for confirmation; lowering it applies from Vibe's next action. A saved higher level also asks for confirmation
when the app starts again.

Closing a tab you opened has its own choice: Settings > Browser > Delegation permissions > Close tabs you opened.
It applies to both your Chrome and Vibe Work's browser. Vault permissions are separate; choosing a Chrome level
does not grant a login.

A Chrome prompt waits until you answer; saying No answers only that call and the turn goes on, and the agent is told
not to do it another way and to wait for your reply (a No on a card for Vibe's own tools stops the turn). Ctrl/Cmd+Shift+A goes to a waiting prompt.
The session header lists session permissions with Revoke. Settings > Allowed without asking lets you review
permissions and find the controls for lasting choices.

## Remove

Settings > Browser > Your Chrome > Remove removes the helper and its registration and saved level. Chrome, its
tabs and the extension remain. Remove the extension separately in Chrome.
