# components

Shared presentational components (props in, markup out, no business logic). Feature-specific
components live in `../features/<feature>/`.

`markdown/Markdown.tsx` is the ONE renderer for untrusted text (no raw HTML, no images, links never
navigate); see its header.

`Selector.tsx` is the pick-one control for Settings, Vault and composer model/thinking choices.
Use `compact` where space is limited; option descriptions, disabled choices and permission dots
share the same trigger and keyboard menu. Navigation tabs, checkboxes and action buttons retain
their own semantics.

`RowMenu.tsx` provides action menus at a pointer or keyboard opener for sessions, projects and files.
It handles window edges, arrow keys, Escape and focus return; chooser rows can add a sibling action.

`Controls.tsx` provides `Button`, `IconButton`, `LinkButton`, `Badge` and `RowDisclosure`.
Their fixed classes keep visual decisions in shared CSS. Settings/Vault lint rejects unlisted
raw controls and local font declarations.
