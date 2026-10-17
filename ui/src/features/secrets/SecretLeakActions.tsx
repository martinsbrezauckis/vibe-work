/** Existing bell action buttons for Rust-owned findings. Depends on scan state; navigation opens existing surfaces. */
import { Button } from "../../components/Controls";
import { useStore } from "../../state/createStore";
import { useApp } from "../shell/AppContext";

/** Completed entries have no finding actions; passwords always retain their manual choice. */
export function SecretLeakActions({ findingId }: { findingId: string }) {
  const app = useApp();
  const scan = app.secretScan;
  return scan === null ? null : <Actions findingId={findingId} scan={scan} />;
}

function Actions({
  findingId,
  scan,
}: {
  findingId: string;
  scan: NonNullable<ReturnType<typeof useApp>["secretScan"]>;
}) {
  const app = useApp();
  const { view, pending, error } = useStore(scan.store, (state) => state);
  const finding = view?.findings.find((item) => item.findingId === findingId);
  if (!finding || finding.state === "removed") return null;
  const logins = [
    ...new Set(
      finding.secrets
        .filter((name) => name.websitePassword && name.state !== "removed")
        .flatMap((name) => (name.loginId === null ? [] : [name.loginId])),
    ),
  ];
  return (
    <>
      <Button
        small
        data-testid="secrets.finding-redact"
        data-item-id={findingId}
        disabled={pending.includes(findingId)}
        onClick={() => {
          void scan.run({ op: "redact", findingId });
        }}
      >
        Redact
      </Button>
      <Button
        small
        data-testid="secrets.finding-open-chat"
        data-item-id={findingId}
        onClick={() => {
          app.shell.actions.closeSettings();
          app.projects?.close();
          app.connectors?.actions.close();
          app.libraries?.actions.close();
          void app.sessions?.open(finding.chatId);
          app.toasts.close();
        }}
      >
        Open chat
      </Button>
      {logins.map((id) => (
        <Button
          small
          key={id}
          data-testid="secrets.finding-change-password"
          data-item-id={id}
          onClick={() => {
            app.projects?.close();
            app.connectors?.actions.close();
            app.shell.actions.openSettings(
              "secrets",
              `vault-login-replace-${encodeURIComponent(id)}`,
            );
            app.toasts.close();
          }}
        >
          Change password
        </Button>
      ))}
      {error !== null && (
        <span className="key-problem" role="alert">
          {error}
        </span>
      )}
    </>
  );
}
