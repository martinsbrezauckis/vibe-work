1. First, load `vw:handbook` with the skill tool and read it before doing anything else.
2. Read the repository's README and main folders to understand the project.
3. If this chat has no task yet, ask exactly one question: what work do I want to plan? Wait for my answer before writing the plan.
4. Create `docs/handbook/` (so a legacy `docs/pages/` does not receive the stylesheet), then call `handbook_check({repair: "stylesheet"})` to install the shared stylesheet; never write CSS yourself. Report any stylesheet conflict to me and stop. Write `index.html`, `roadmap.html`, `changelog.html` and the task's first page in `plans/`. Follow the skill's format, metadata, steps and links; link every page from the overview.
5. This request is only to create the handbook and plan. Leave code and `AGENTS.md` unchanged; do not ask other questions or start implementing the task. Keep its planned steps `todo`.
6. Run `handbook_check`, fix all errors, list the pages created and stop.
