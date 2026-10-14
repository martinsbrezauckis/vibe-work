1. First, load `vw:handbook` with the skill tool and read it before doing anything else.
2. Read the existing handbook and the repository's README and main folders.
3. If this chat has no task yet, ask exactly one question: what work do I want to plan? Wait for my answer before writing the plan.
4. Call `handbook_check({repair: "stylesheet"})` to install or repair the shared stylesheet; never write CSS yourself. Report any stylesheet conflict to me and stop. Write the task's first page in `docs/handbook/plans/`. Follow the skill's format, metadata and steps, and link the plan from `index.html`.
5. This request is only to write the plan. Leave code and `AGENTS.md` unchanged; do not ask other questions or start implementing the task. Keep its planned steps `todo`.
6. Run `handbook_check`, fix all errors, list the pages created or updated and stop.
