# Linux desktop accessibility pilot

Standalone Rust library for AT-SPI2 accessibility observations and one action followed by verification.
Depends on `atspi`'s typed zbus proxies; called directly by Rust clients and tests. Run on a Tokio runtime.

`Desktop::connect()` discovers the accessibility bus through `org.a11y.Bus` on the current session's
D-Bus. `Desktop::connect_at(address)` uses an explicit address supplied by a host or isolated test.
This pilot exposes its API only on Linux.

```rust,no_run
use vw_desktop::{Action, Desktop, Postcondition};

async fn example() -> Result<(), Box<dyn std::error::Error>> {
    let desktop = Desktop::connect().await?;
    let tree = desktop.snapshot(Some("Desktop pilot fixture")).await?;
    let button = tree.elements.iter().find(|e| e.name == "Press me").ok_or("button absent")?;
    let receipt = desktop.act(&button.id, Action::Press {
        expected: Postcondition::Name("Pressed 1".into()),
    }).await?;
    assert!(receipt.is_success());

    let field = tree.elements.iter().find(|e| e.name == "Message").ok_or("field absent")?;
    let typed = desktop.act(&field.id, Action::SetText("Hello".into())).await?;
    assert!(typed.is_success());
    let verified = desktop.verify(&field.id, &Postcondition::Text("Hello".into())).await?;
    assert!(verified.matched);
    Ok(())
}
```

`snapshot(None)` selects the top-level window reporting Active or Focused. An exact window name
selects a unique Frame, Dialog or Window among registered applications' immediate children.
Missing, ambiguous or incomplete discovery returns an error. The preorder list includes role,
accessible name, states, screen bounds when Component exists, depth and the stable identity pair
`{ bus, path }`. Identity survives renaming/reordering while that accessible object lives; it is not
persistent across application or bus restarts. StateSet supports `contains(State::Focused)` and
other named AT-SPI states; serde uses the standard AT-SPI state bit words.

Snapshots have at most 256 elements, depth 32, 512 bytes per name and 1024 bytes per identity.
Tree omission sets `Snapshot::truncated`; each shortened name sets `name_truncated`.
Text requests and returned text have a 4096-byte ceiling. UTF-8 prefixes are explicitly marked
incomplete and never prove exact equality. Discovery inspects at most 256 application/window entries.

Press invokes Action index zero, the default action; its caller supplies the observable effect on
the same element. SetText invokes EditableText once and verifies exact Text contents. Focus invokes
Component::GrabFocus once and verifies Focused. All actions check live sensitivity and the applicable
editable/focusable state. Each dispatch has exactly one mutation, followed by uncached property reads.
`Receipt::is_success()` requires method acceptance and a matching postcondition; an unmet effect is
returned with its actual observation. No action is retried. A transport failure after dispatch can
leave the effect uncertain, so inspect it before considering a manual retry. Verification proves
the state at the read, not permanent application behavior; asynchronous effects can be checked later
with the read-only `verify` call.

Display-free tests, including a recorded GTK observation replay:

```sh
CARGO_BUILD_JOBS=4 cargo test -p vw-desktop
```

The live test is opt-in and creates its own GTK fixture, Xvfb display and session bus:

```sh
sudo apt-get install xvfb xauth dbus at-spi2-core libatk-adaptor gir1.2-gtk-3.0 python3-gi
bash crates/vw-desktop/tests/run-demo.sh
```

For a container whose D-Bus cannot query the host's AppArmor policy, use
`bash crates/vw-desktop/tests/run-demo.sh --isolated-bus`. This launches a fixture-only bus using
`tests/demo-bus.conf`, disables AppArmor mediation only on that new bus, activates the real AT-SPI
registry, and supplies its address to GTK and `connect_at`. It changes no installed bus configuration.
Xvfb runs with GLX disabled; these test controls need no GPU. Missing prerequisites fail the demo.

There are no agent tools, app/UI integration, permissions UX, planner, image capture, coordinate input,
vision fallback or non-Linux implementations in this crate.
