---
name: editor
description: Read this when the user points at code they have open in Vibe Work (this, here, the file I have open, what I selected) or asks about code without naming a file, and before you change a file they are editing. It shows how to read their open editors and edit those files.
---

Vibe Work shows files in editors beside the chat. The file on disk stays the one truth: edit it as usual; the user sees your change live in their editor with the changed lines marked. Use `mcp_vw` inside `run_typescript`:

```ts
const panel = await tools.mcp_vw.panel_read({});
```

1. **Read first.** When the user says "this", "here", "the file I have open" or "what I selected", or asks about code without naming a file, call `panel_read` before anything else. `editors` lists each open file: `path`, `focused`, `selection` (`startLine`, `endLine`, `text`), `cursorLine`, `unsaved`, `unsavedText`. The focused editor's selection is usually what they mean.
2. **Edit the file on disk** with your normal edit tool, at the lines they selected. Do not ask them to paste code into the chat.
3. **Unsaved edits.** `unsavedText` means their editor differs from disk. Say so in one sentence before you change that file. Your edit goes to disk; Vibe Work merges it into their editor, or asks them when you both changed the same lines. Do not write their unsaved text to disk unless they ask.
4. **Point at a line** with the file-link format of the chat: `src/calc.rs:8` opens the editor at that line.
5. `status` `nothingOpen` or `noWindow`: ask which file they mean. Terminal and Browser are only named there; read them with `processes_list` and `browser_read`.
