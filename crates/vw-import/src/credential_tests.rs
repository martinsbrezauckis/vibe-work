//! Written-out credentials in a server's address, headers and environment never reach the plan or Vibe's config.
//! Fake values only. A canonical fixture root keeps OS temp-directory aliases out of these paths.

use super::fs::Disk;
use super::mcp::Server;
use super::{Agent, Found, Homes, Kind, Payload, Status, Taken, scan, scan_with_payloads};

const VALUE: &str = "fake-credential-value-for-tests-6b1d";

fn home_with(servers: &str) -> (tempfile::TempDir, Homes) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let home = root.join("home");
    let path = home.join(".cursor/mcp.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!(r#"{{"mcpServers":{{{servers}}}}}"#)).unwrap();
    let homes = Homes {
        home,
        codex_home: None,
        config_dir: root.join("config"),
        project: None,
    };
    (dir, homes)
}

fn server<'a>(rows: &'a [Found], name: &str) -> &'a Found {
    rows.iter()
        .find(|r| r.agent == Agent::Cursor && r.kind == Kind::McpServer && r.name == name)
        .unwrap_or_else(|| panic!("no server {name}: {rows:#?}"))
}

// PREDICT: `detail` split the address at its first `/` before the query, so with no path
// (`https://example.test?token=V`) the "host" was `example.test?token=V` and the plan carried the value, although the
// row was Can't bring. With the display built from the address's parts (fragment, then query, then authority) and
// the whole query masked, no serialized field of any row holds the value.
#[test]
fn an_address_credential_never_reaches_any_field_of_the_plan() {
    let (_dir, homes) = home_with(&format!(
        r#""bare":{{"url":"https://example.test?token={VALUE}"}},
           "pathless":{{"url":"https://example.test?region=eu&api_key={VALUE}#x"}},
           "user":{{"url":"https://me:{VALUE}@example.test?q=1"}},
           "plain":{{"url":"https://example.test/mcp?region=eu"}}"#
    ));
    let rows = scan(&Disk, &homes, &Taken::default());
    let shown = serde_json::to_string(&rows).unwrap();
    assert!(!shown.contains(VALUE), "a value reached the plan: {shown}");
    for name in ["bare", "pathless", "user"] {
        assert_eq!(server(&rows, name).status, Status::CantBring, "{name}");
    }
    assert_eq!(
        server(&rows, "bare").detail.as_deref(),
        Some("https://example.test/?\u{2022}\u{2022}\u{2022}")
    );
    assert_eq!(
        server(&rows, "user").detail.as_deref(),
        Some("https://example.test/?\u{2022}\u{2022}\u{2022}")
    );
    // The whole query is masked, also where no name says key.
    assert_eq!(
        server(&rows, "plain").detail.as_deref(),
        Some("https://example.test/mcp?\u{2022}\u{2022}\u{2022}")
    );
}

// PREDICT: only settings whose name says key (`*TOKEN*`, `Authorization`, ...) counted as keys, so a
// literal credential under `X-Auth` (header) or `MCP_AUTH` (environment) made a Ready row whose value went into
// Vibe's config as plain text, and `MCP_AUTH`'s value was shown in the plan. With every written-out header or
// environment value counted as a key: one header goes to the vault (permission Ask, the forwarder carries it), more
// than one header or any program environment value is Can't bring, and no value is shown.
#[test]
fn a_written_out_header_or_environment_value_is_a_key_whatever_its_name() {
    let (_dir, homes) = home_with(&format!(
        r#""xauth":{{"url":"https://api.example.test/mcp","headers":{{"X-Auth":"{VALUE}"}}}},
           "two":{{"url":"https://api.example.test/mcp","headers":{{"Authorization":"Bearer {VALUE}","X-Client":"vibe"}}}},
           "envauth":{{"command":"tool","args":["serve"],"env":{{"MCP_AUTH":"{VALUE}"}}}},
           "byref":{{"command":"tool","args":["serve"],"env":{{"MCP_AUTH":"${{MCP_AUTH}}"}}}},
           "refheader":{{"url":"https://api.example.test/mcp","headers":{{"X-Auth":"${{X_AUTH}}"}}}}"#
    ));
    let planned = scan_with_payloads(&Disk, &homes, &Taken::default());
    let rows: Vec<Found> = planned.iter().map(|(f, _)| f.clone()).collect();
    let shown = serde_json::to_string(&rows).unwrap();
    assert!(!shown.contains(VALUE), "a value reached the plan: {shown}");

    let xauth = server(&rows, "xauth");
    assert_eq!(xauth.status, Status::Changed, "{xauth:?}");
    assert_eq!(xauth.keys, ["X-Auth"]);
    // PREDICT: the row's note repeated the vault line the Settings row already shows for every row with a key
    // ("Key goes to your vault — Key goes to your vault; ..."); the row now carries no note and the UI shows the one
    // vocabulary sentence (`importKeyToVault`).
    assert_eq!(xauth.note, None, "{xauth:?}");
    assert_eq!(server(&rows, "two").status, Status::CantBring);
    let envauth = server(&rows, "envauth");
    assert_eq!(envauth.status, Status::CantBring, "{envauth:?}");
    assert_eq!(envauth.keys, ["MCP_AUTH"]);
    // A reference to the person's own environment is not a value: brought as written.
    assert_eq!(server(&rows, "byref").status, Status::Ready);
    assert_eq!(server(&rows, "refheader").status, Status::Ready);
    // Every row that is brought as plain config holds no written-out header or environment value.
    for (found, payload) in &planned {
        if found.status != Status::Ready {
            continue;
        }
        let Payload::Server(server) = payload else {
            continue;
        };
        let pairs = match server {
            Server::Stdio { env, .. } => env,
            Server::Http { headers, .. } => headers,
        };
        assert!(
            pairs.iter().all(|(_, v)| !super::mcp::literal(v)),
            "{} would write a value into Vibe's config",
            found.name
        );
    }
}
