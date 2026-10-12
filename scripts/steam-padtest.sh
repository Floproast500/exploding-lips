#!/bin/sh
# The Steam entry (non-Steam game) for playing with the Steam Controller: starts the remaster
# through play.sh, logging controller input to re/run/play.log. Started from Steam, the
# controller gets the gamepad layout; the game runs on X11 so Steam sees it has focus.
export RUST_LOG="${RUST_LOG:-info,lips_desktop::gamepad=trace}"
exec "$(dirname "$0")/play.sh" "$@"
