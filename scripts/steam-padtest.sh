#!/bin/sh
# Controller check under Steam Input: add this script to Steam as a non-Steam game and launch it
# from Steam, so the controller gets a gamepad layout instead of the desktop layout. Starts
# LIPS00 and logs every gamepad button and stick input to re/run/padtest.log.
# The window is forced onto X11 (XWayland): Steam cannot see focus of native Wayland windows
# and would keep applying its desktop (keyboard/mouse) layout.
cd "$(dirname "$0")/.."
mkdir -p re/run
unset WAYLAND_DISPLAY
RUST_LOG=info,lips_desktop::gamepad=trace exec target/release/lips-desktop --data original --level LIPS00 \
    > re/run/padtest.log 2>&1
