---
name: editor
description: Vibe Work's editors beside the chat - panel_read returns the user's open files, selection, cursor and unsaved edits (this, here, the file I have open, what I selected), and how file edits reach an open editor.
---

Vibe Work shows files in editors beside the chat. The file on disk stays the one truth: a change to it appears live in the user's editor with the changed lines marked. `panel_read` is on `mcp_vw` inside `run_typescript`:

```ts
const panel = await tools.mcp_vw.panel_read({});
```

1. `editors` lists each open file. Allowed reads have `path`, `focused`, `selection` (`startLine`, `endLine`, `text`), `cursorLine`, `unsaved`, `unsavedText`. If file-read rules deny access or cannot be checked, the entry has only `path`, `status: "withheld"` and `reason`. The focused editor's selection is what the user has selected now.
2. `unsavedText` means the user's editor differs from disk. An edit on disk is merged into the open editor; when both changed the same lines, Vibe Work asks the user which to keep. Settings > General > Save open files before Vibe works (on by default) saves open files first.
3. A file link in the chat's format (`src/calc.rs:8`) opens the editor at that line.
4. `status` `nothingOpen` or `noWindow`: no editor is open, or no window shows this chat. Terminal and Browser are only named there; `processes_list` and `browser_read` read them.
5. Files previews cannot run scripts, including scripts in handbook prototypes.
