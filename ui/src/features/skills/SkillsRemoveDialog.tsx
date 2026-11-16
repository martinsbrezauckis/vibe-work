/** Destructive skill confirmation with Cancel focused and no implicit Enter submission. */
import { Button } from "../../components/Controls";
import { Modal } from "../../components/Modal";
import { useApp } from "../shell/AppContext";
import { useSkills } from "./hooks";
/** Remove confirmation for the skill chosen for removal; renders nothing while none is. */
export function SkillsRemoveDialog() {
  const { skills } = useApp();
  const name = useSkills((s) => s.removing);
  const pending = useSkills((s) => s.pending);
  const busy = useSkills((s) => s.view?.busy === true);
  const close = () => skills?.actions.showRemove(null);
  if (name === null) return null;
  return (
    <Modal
      onClose={close}
      scrimTestId="skills.remove-scrim"
      data-testid="skills.remove-dialog"
      aria-labelledby="skills-remove-title"
    >
      <header>
        <h2 id="skills-remove-title">Remove {name}?</h2>
      </header>
      <section>
        <p>Vibe stops offering it.</p>
      </section>
      <section className="skills-actions">
        <Button data-testid="skills.remove-cancel" data-dialog-initial-focus="true" onClick={close}>
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
    </Modal>
  );
}
