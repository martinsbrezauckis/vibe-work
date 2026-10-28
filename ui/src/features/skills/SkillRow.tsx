/**
 * One skill's disclosure and actions. Install… and Update to vN… open the row's detail with the install offer
 * (`InstallOffer`): its button installs exactly what it shows. Shared controls own their appearance; the
 * skills store and its offer flow own changes and focus.
 * Depends on: Controls, InstallOffer, the skills hooks. Called by: SkillsPage.
 */
import { builtinSkillText } from "../../state/builtinToolsText";
import { useRef } from "react";
import { Badge, Button, LinkButton, RowDisclosure, Switch } from "../../components/Controls";
import { Markdown } from "../../components/markdown/Markdown";
import { PathText } from "../../components/PathText";
import { Icon } from "../../components/Icon";
import type { SkillRow as Row } from "../../protocol/core";
import { useApp } from "../shell/AppContext";
import { term } from "../../vocabulary";
import { SKILLS_BUSY } from "../../state/skills";
import { InstallOffer, useOfferFocus } from "../connectors/InstallOffer";
import { useSkillOffers, useSkills } from "./hooks";
const SOURCES: Readonly<Record<string, string>> = {
  builtin: "Built-in",
  user: "Yours",
  project: term("workNames", "projectTab").name,
  plugin: "Plugin",
  catalog: "Catalog",
};

/** A name opens the full description, versions and source below its row. */
export function SkillRow({ row }: { row: Row }) {
  const { skills } = useApp();
  const display = builtinSkillText(row);
  const s = useSkills((state) => state);
  const expanded = s.expanded.includes(row.name);
  const pending = s.pending.length > 0;
  const disabled = row.locked || s.view?.busy === true || pending;
  const reason = row.lockReason ?? (s.view?.busy ? SKILLS_BUSY : undefined);
  const detail = s.details[row.name];
  const id = `skill-detail-${encodeURIComponent(row.name)}`;
  const offer = useSkillOffers((o) => o.open[row.name]);
  const problem = useSkillOffers((o) => o.problems[row.name]);
  const focus = useSkillOffers((o) => o.focus);
  const opener = useRef<HTMLButtonElement>(null);
  useOfferFocus(focus, row.name, "opener", opener);
  return (
    <article className="ui-list-row skills-row" data-item-id={row.name}>
      <div className="skills-row-main">
        <div className="skills-row-text">
          <div className="skills-row-title">
            <span className="skills-name">
              <RowDisclosure
                data-testid="skills.disclose"
                data-skill-name={row.name}
                aria-expanded={expanded}
                aria-controls={id}
                onClick={() => void skills?.actions.disclose(row.name)}
              >
                <Icon name={expanded ? "chevron-down" : "chevron-right"} size={14} />
                <span>{display?.name ?? row.name}</span>
              </RowDisclosure>
            </span>
            {row.source === null ? null : <Badge>{SOURCES[row.source]}</Badge>}
          </div>
          <p className="skills-description">{display?.description ?? row.description}</p>
          {row.lockReason === null ? null : <p className="cmuted">{row.lockReason}</p>}
        </div>
        <div className="skills-actions">
          <span className="cmuted">v{row.installedVersion ?? row.latestVersion ?? "—"}</span>
          {row.installed ? (
            <>
              {row.updateAvailable && row.latestVersion !== null && offer === undefined ? (
                <Button
                  data-testid="skills.update"
                  ref={opener}
                  shape="outlined"
                  disabled={disabled}
                  title={reason}
                  onClick={() => void skills?.actions.offerUpdate(row.name)}
                >
                  Update to v{row.latestVersion}…
                </Button>
              ) : null}
              <Switch
                data-testid="skills.toggle"
                aria-label={`Use ${row.name}`}
                aria-checked={row.enabled === true}
                disabled={disabled || row.enabled === null}
                title={reason}
                onClick={() =>
                  void skills?.actions.change(
                    { op: "setEnabled", name: row.name, enabled: !row.enabled },
                    row.name,
                  )
                }
              />
              {row.skillId === null ? null : (
                <LinkButton
                  data-testid="skills.remove"
                  variant="danger"
                  disabled={disabled}
                  title={reason}
                  onClick={() => skills?.actions.showRemove(row.name)}
                >
                  Remove…
                </LinkButton>
              )}
            </>
          ) : s.view?.authenticated && row.skillId !== null && offer === undefined ? (
            <Button
              data-testid="skills.install"
              ref={opener}
              shape="outlined"
              disabled={disabled}
              title={reason ?? "For all projects"}
              onClick={() => void skills?.actions.offerInstall(row.name)}
            >
              Install…
            </Button>
          ) : null}
        </div>
      </div>
      {s.errors[row.name] === undefined ? null : (
        <p className="key-problem" role="alert">
          {s.errors[row.name]}
        </p>
      )}
      {problem === undefined ? null : (
        <p className="key-problem" role="alert" data-testid="skills.offer-problem">
          {problem}
        </p>
      )}
      {offer !== undefined ? (
        <div className="skills-detail" id={id}>
          <InstallOffer
            testId="skills.offer"
            open={offer}
            focus={focus}
            onConfirm={() => void skills?.actions.confirmOffer(row.name)}
            onCancel={() => {
              skills?.actions.cancelOffer(row.name);
            }}
          />
        </div>
      ) : expanded ? (
        <div className="skills-detail" id={id}>
          <p>{display?.description ?? detail?.description ?? row.description}</p>
          {display ? (
            <p className="cmuted">
              <code>{row.name}</code>
            </p>
          ) : null}
          <p className="cmuted">
            Source: {row.source === null ? "Unavailable" : SOURCES[row.source]}
          </p>
          {s.detailLoading.includes(row.name) ? (
            <p role="status" className="cmuted">
              Reading skill details…
            </p>
          ) : null}
          {detail?.locations.map((location) => (
            <p className="cmuted" key={location}>
              <PathText path={location} />
            </p>
          ))}
          {detail?.versions.length ? (
            <p className="cmuted">Versions: {detail.versions.join(", ")}</p>
          ) : null}
          {detail?.body ? (
            <div className="a-msg">
              <Markdown source={detail.body} />
            </div>
          ) : null}
        </div>
      ) : null}
    </article>
  );
}
