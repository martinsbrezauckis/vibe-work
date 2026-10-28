/** Destructive skill confirmation with Cancel focused and no implicit Enter submission. */
import { useRef } from "react";
import { Button } from "../../components/Controls";
import { useModalFocus } from "../../hooks/useModalFocus";
import { useApp } from "../shell/AppContext";
import { useSkills } from "./hooks";
export function SkillsRemoveDialog() {
  const { skills } = useApp();
  const name = useSkills((s) => s.removing);
  const pending = useSkills((s) => s.pending);
  const busy = useSkills((s) => s.view?.busy === true);
  const ref = useRef<HTMLDivElement>(null);
  const close = () => skills?.actions.showRemove(null);
  useModalFocus(name !== null, ref, close);
  if (name === null) return null;
  return (
    <>
      <div className="scrim" data-testid="skills.remove-scrim" onClick={close} />
      <div
        className="modal"
        data-testid="skills.remove-dialog"
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="skills-remove-title"
        tabIndex={-1}
      >
        <header>
          <h2 id="skills-remove-title">Remove {name}?</h2>
        </header>
        <section>
          <p>Vibe stops offering it.</p>
        </section>
        <section className="skills-actions">
          <Button
            data-testid="skills.remove-cancel"
            data-dialog-initial-focus="true"
            onClick={close}
          >
            Cancel
          </Button>
          <Button
            data-testid="skills.remove-confirm"
            variant="danger"
            disabled={busy || pending.includes(name)}
            onClick={() => void skills?.actions.change({ op: "remove", name }, name)}
          >
            Remove
          </Button>
        </section>
      </div>
    </>
  );
}
