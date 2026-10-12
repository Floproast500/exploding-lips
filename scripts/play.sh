#!/bin/sh
# Play the remaster: GPU renderer, HD texture pack (when built), SDL controllers with gyro.
# For the Steam Controller, add this script to Steam (Games > Add a Non-Steam Game > Browse,
# pick this file) and start it from Steam: Steam then gives the controller the game's gamepad
# layout. The game uses X11 under Steam, because Steam cannot see the focus of Wayland windows.
cd "$(dirname "$0")/.."
[ -x target/release/biglips ] && [ -x target/release/packbuilder ] || cargo build --release -p lips-desktop -p packbuilder --features lips-desktop/sdl || exit 1
pack=""
[ -f packs/lips-hd/pack.txt ] && pack="--set r_texpack packs/lips-hd"
# Outside Steam, prefer X11 too (Steam may still be running and grabbing the controller).
unset WAYLAND_DISPLAY
mkdir -p re/run
exec target/release/biglips --data original --set r_classic 0 $pack "$@" > re/run/play.log 2>&1
