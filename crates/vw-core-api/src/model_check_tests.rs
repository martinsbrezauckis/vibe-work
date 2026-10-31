//! Launch-check preservation tests over disposable config files and a Python stdio lobby.
//! Depends on Python 3 and the core dispatcher; no network, desktop or model is used.

use serde_json::json;
use std::sync::Arc;
use vw_gate::{Confirmation, ConfirmationRequest, Confirmer};

struct NoPrompt;
impl Confirmer for NoPrompt {
    async fn confirm(&self, _: &ConfirmationRequest) -> Confirmation {
        panic!("the launch check must not prompt");
    }
}

const LOBBY: &str = r"#!/usr/bin/env python3
import json, os, pathlib, sys
home = pathlib.Path(os.environ['VIBE_HOME'])
for line in sys.stdin:
    frame = json.loads(line)
    message = frame['message']
    if message is None:
        print(json.dumps(frame), flush=True)
        continue
    if 'id' not in message:
        continue
    if message['method'] == 'initialize':
        result = {'serverInfo': {'name': 'model-check-test', 'version': '0'}}
    elif message['method'] == 'config/read':
        (home / 'read-config.toml').write_bytes((home / 'config.toml').read_bytes())
        result = {'config': {'models': json.loads((home / 'rows.json').read_text())}}
    else:
        raise AssertionError(message['method'])
    frame['message'] = {'jsonrpc': '2.0', 'id': message['id'], 'result': result}
    print(json.dumps(frame), flush=True)
";

#[tokio::test]
async fn a_different_alias_with_our_prices_survives_the_launch_check_byte_for_byte() {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::temp_dir().join(format!("vw-model-check-{}", std::process::id()));
    let home = root.join("vibe");
    std::fs::create_dir_all(&home).unwrap();
    let program = root.join("lobby.py");
    std::fs::write(&program, LOBBY).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let core = crate::Core::start(
        vw_supervisor::SupervisorConfig::new(program, home.clone(), root.clone()),
        Arc::new(vw_supervisor::NoSecret),
        NoPrompt,
        crate::CoreConfig {
            instructions: crate::InstructionsConfig {
                cli_home: Some(root.join("cli")),
            },
            ..crate::CoreConfig::default()
        },
    )
    .await
    .unwrap();
    let file = home.join("config.toml");
    for offered in [false, true] {
        let rows = if offered {
            json!([{"alias":"mistral-large-4-0-3", "name":"mistral-large-4-0",
                "source":"growthbook", "displayName":"Mistral Large 4"}])
        } else {
            json!([])
        };
        std::fs::write(home.join("rows.json"), rows.to_string()).unwrap();
        for window in ["", "max_context_length = 1048576\n"] {
            let original = format!(
                "# My prices\n[[models]]\nalias = 'my-model'\n{window}input_price = 0.70 # keep this\ncached_input_price = 0.07\noutput_price = 2.10\n"
            );
            std::fs::write(&file, &original).unwrap();
            for _ in 0..2 {
                let result = core
                    .execute(crate::Origin::Ui, crate::Command::CheckDefaultModel {})
                    .await
                    .unwrap();
                assert!(matches!(result, crate::CommandResult::Reply { .. }));
                let after = std::fs::read_to_string(&file).unwrap();
                let read = std::fs::read_to_string(home.join("read-config.toml")).unwrap();
                assert!(
                    read.starts_with(&original),
                    "the lobby read lost user bytes: {read:?}"
                );
                if offered {
                    assert_eq!(
                        crate::model_facts::without(&after).as_deref(),
                        Some(original.as_str())
                    );
                } else {
                    assert_eq!(after, original, "the launch check changed user bytes");
                }
            }
        }
    }
    core.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}
