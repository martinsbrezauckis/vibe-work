/** The Tools skills list. Depends on shared controls and the skills controller; no transport logic. */
import { builtinSkillText } from "../../state/builtinToolsText";
import { useEffect, useRef } from "react";
import { Button, LinkButton } from "../../components/Controls";
import { ToolsTabs } from "../connectors/ToolsTabs";
import { useApp } from "../shell/AppContext";
import { SKILLS_BUSY } from "../../state/skills";
import { SkillRow } from "./SkillRow";
import { SkillsRemoveDialog } from "./SkillsRemoveDialog";
import { useSkills } from "./hooks";
import "./skills.css";

/** Third Tools tab, with installed and uninstalled catalog groups. */
export function SkillsPage() {
  const { skills, shell } = useApp();
  const s = useSkills((state) => state);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => skills?.watch(), [skills]);
  useEffect(() => {
    if (s.focus === null) return;
    const row = Array.from(
      root.current?.querySelectorAll<HTMLButtonElement>("[data-skill-name]") ?? [],
    ).find((b) => b.dataset["skillName"] === s.focus?.name);
    (
      row ??
      root.current?.querySelector<HTMLElement>(
        s.focus.installed ? "#skills-installed" : "#skills-catalog",
      )
    )?.focus();
  }, [s.focus]);
  const view = s.view;
  const query = s.search.trim().toLowerCase();
  const all = view?.skills ?? [];
  const filtered = all.filter((r) =>
    `${builtinSkillText(r)?.name ?? ""} ${builtinSkillText(r)?.description ?? ""} ${r.name} ${r.description}`
      .toLowerCase()
      .includes(query),
  );
  return (
    <div className="cpage" data-testid="skills-page" ref={root}>
      <div className="cmain">
        <div className="chead">
          <ToolsTabs />
          <p className="cmuted">
            Skills Vibe can use in chats. Changes apply to the selected chat and new chats in this
            folder.
          </p>
        </div>
        {all.length > 8 || s.search !== "" ? (
          <div className="cbar">
            <label className="csearch">
              <input
                data-testid="skills.search"
                aria-label="Search skills"
                placeholder="Search skills"
                value={s.search}
                onChange={(e) => skills?.actions.setSearch(e.target.value)}
              />
            </label>
          </div>
        ) : null}
        {s.error === null ? null : (
          <p className="server-note" role="alert">
            Vibe could not list its skills: {s.error.replace(/\.$/, "")}.{" "}
            <Button
              data-testid="skills.retry"
              shape="outlined"
              onClick={() => void skills?.actions.load()}
            >
              Try again
            </Button>
          </p>
        )}
        {view?.issues.length ? (
          <div className="server-note" role="status">
            {view.issues.map((i) => (
              <p key={`${i.file}:${i.message}`}>
                {i.file}: {i.message}
              </p>
            ))}
          </div>
        ) : null}
        {view?.busy ? (
          <p className="server-note" role="status">
            {SKILLS_BUSY}
          </p>
        ) : null}
        {s.note === null ? null : (
          <p className="cnote" role="status">
            {s.note}
          </p>
        )}
        {[true, false].map((installed) => {
          const rows = filtered.filter((r) => r.installed === installed);
          const count = all.filter((r) => r.installed === installed).length;
          return (
            <section
              className="cgroup"
              key={String(installed)}
              data-item-id={installed ? "installed" : "catalog"}
              aria-labelledby={installed ? "skills-installed" : "skills-catalog"}
            >
              <h3
                data-testid="skills.group"
                id={installed ? "skills-installed" : "skills-catalog"}
                tabIndex={-1}
              >
                {installed ? "On this computer" : "Mistral's catalog"} · {count}
              </h3>
              {s.loading && view === null ? (
                <div aria-label="Loading skills" role="status" className="skills-loading">
                  {[0, 1, 2].map((n) => (
                    <div className="sk" key={n} />
                  ))}
                </div>
              ) : null}
              {!installed && view !== null && !view.authenticated ? (
                <p className="server-note">
                  Add your Mistral key to install skills from Mistral's catalog.{" "}
                  <LinkButton
                    data-testid="skills.account"
                    onClick={() => {
                      shell.actions.openSettings("account");
                    }}
                  >
                    Open Account settings
                  </LinkButton>
                </p>
              ) : !installed && view !== null && (!view.catalogLoaded || count === 0) ? (
                <p className="server-note">
                  {view.catalogLoaded
                    ? "Mistral's catalog has no skills to install right now."
                    : "Mistral's catalog is not available right now."}
                </p>
              ) : null}
              <div className="skills-list">
                {rows.map((row) => (
                  <SkillRow key={row.name} row={row} />
                ))}
              </div>
              {query !== "" && rows.length === 0 ? (
                <p className="cmuted">No skills match.</p>
              ) : null}
            </section>
          );
        })}
      </div>
      <SkillsRemoveDialog />
    </div>
  );
}
