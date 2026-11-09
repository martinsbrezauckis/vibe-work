/** Quiet action for explicitly classified app failures. Opens the shared editable Bug form; never attaches or sends. */
import { Button } from "../../components/Controls";
import { feedbackFor } from "../../state/feedback";
import { FEEDBACK_ENABLED } from "../../state/feedbackEnabled";
import type { CoreController } from "../../state/coreStore";
import { useStore } from "../../state/createStore";

/** Bound prose to the core's UTF-8 limits without cutting a character. */
function bounded(text: string, limit: number): string {
  let bytes = 0;
  const encoder = new TextEncoder();
  return Array.from(text)
    .filter((character) => {
      bytes += encoder.encode(character).length;
      return bytes <= limit;
    })
    .join("");
}

interface Props {
  core: CoreController | null | undefined;
  title: string;
  text: string;
  onOpen?: () => void;
}
export function FeedbackAction(props: Props) {
  if (!FEEDBACK_ENABLED || !props.core) return null;
  return <ReadyAction {...props} core={props.core} />;
}
function ReadyAction({ core, title, text, onOpen }: Props & { core: CoreController }) {
  const feedback = feedbackFor(core);
  const busy = useStore(feedback.store, (s) => s.busy);
  return (
    <Button
      data-testid="feedback.failure"
      data-item-id={title}
      appearance="expander"
      disabled={busy}
      onClick={() => {
        void feedback
          .open({
            kind: "bug",
            title: bounded(title, 160),
            description: bounded(text, 8000),
            remarks: "",
          })
          .then((opened) => {
            if (opened) onOpen?.();
          });
      }}
    >
      Send feedback…
    </Button>
  );
}
