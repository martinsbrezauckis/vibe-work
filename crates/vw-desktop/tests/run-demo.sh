#!/usr/bin/env bash
# Isolated Linux AT-SPI demo; needs Xvfb, D-Bus and GTK, called by developers and CI explicitly.
set -euo pipefail
cd "$(dirname "$0")/../../.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
unset NO_AT_BRIDGE
export GTK_MODULES=atk-bridge
export GTK_A11Y=always
export GSETTINGS_BACKEND=memory
export GIO_USE_VFS=local
if [[ "${1:-}" == "--isolated-bus" ]]; then
    exec xvfb-run -a -s '-screen 0 1024x768x24 -extension GLX' \
        dbus-run-session --config-file="$PWD/crates/vw-desktop/tests/demo-bus.conf" -- bash -c '
            export AT_SPI_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS"
            export VW_DESKTOP_TEST_BUS="$DBUS_SESSION_BUS_ADDRESS"
            dbus-update-activation-environment AT_SPI_BUS_ADDRESS DISPLAY XAUTHORITY
            exec cargo test -p vw-desktop --test live_bus -- --ignored --nocapture
        '
fi
exec xvfb-run -a -s '-screen 0 1024x768x24 -extension GLX' dbus-run-session -- \
    cargo test -p vw-desktop --test live_bus -- --ignored --nocapture
