# Exploding Lips — engine reimplementation

A clean-room reimplementation of the G.O.D.D. ("WinGodd") engine used by **Exploding Lips**
(Galaxy Games, 1999), written in Rust. It plays the original game data on modern systems —
Linux, Windows, macOS, Steam Deck and the web — with widescreen, high resolutions, true-colour
lighting, mouse look, gamepads and netplay.

**You need your own copy of the game.** No original assets are included in this repository;
the engine reads them from your install folder at runtime.

## Status

| Area | State |
|---|---|
| File formats: XRB levels, XPG pixels, PAL, PIC, BMP, INI scripts, GAM saves | parsed; tested on every shipped file; byte-identical save rewrite; damaged-input robustness tests |
| Renderer: portals, stacked (room-over-room) regions, floors/ceilings, sky, sprites with rotations, masked walls, original lighting | matches the original (golden test: RMSE 0.04 view / 0.06 full screen vs. `lips.exe`) |
| Movement & collision (walk/run/turn/jump/fall/step, see-through floors) | constants from the original; golden-tested against memory probes of `lips.exe` |
| Script VM (`ONTAG`, tags, OS states with thresholds, `XM`/`XT`/`OTEXT`/`XCHANGE`/…) | complete for the shipped INIs, original quirks included |
| World effects: doors, lifts, jump pads, rotating rooms, ramps, rafts, switches, region triggers, scrolling/animated textures | implemented; data-driven tests on the shipped levels |
| Objects: bots (mover scripts), pickups, text, looping/positional sounds and sound sequences, wall/object morphs, flying, TV views, overlays, lightning, switches, timers, counters, teleports, level exits | implemented; data-driven tests |
| Combat: tools 0–9, projectiles, explosions (bomb multi-hit), bot attacks, first-person weapon | implemented; shots-to-kill and fire rate confirmed against `lips.exe` |
| HUD, status messages, menus (LIPSTART), character save slots (LIPSTART.GAM) | exact original layout |
| Desktop: wgpu palette shader + CPU fallback, mouse, gamepad/Steam Deck, audio (WAV + MIDI via SoundFont) | Linux tested; Windows build cross-compiled and run under Wine; macOS type-checked; gamepad tested end to end with a virtual Xbox 360 pad (`tools/pilot/gamepad_pilot.py`, uinput) |
| Web (WASM): folder picker, saves in localStorage | working, headless-tested |
| Netplay: UDP host/join, WebSocket for browsers, shared doors/switches, remote players | working |
| Modern extras: Hor+ widescreen, FOV, true colour, F2 automap, quicksave, console, key rebinding, high-contrast HUD, long messages, mods | working |

Reverse-engineering notes live in [`re/`](re/): file formats in `re/formats/`, engine behaviour
in `re/engine/` (renderer, physics, script, movers, combat, HUD, object and world effects).
Engine code is written from those specs, not from decompiler output.

## Layout

```
crates/godd-formats   parsers for the data files (no platform dependencies)
crates/godd-game      engine core: world, renderer, physics, scripts, effects, objects, UI
crates/godd-net       netplay protocol, UDP/WebSocket server, client glue (feature "game")
crates/lips-desktop   native front end (winit + wgpu, kira audio, gilrs gamepads)
crates/lips-web       WebAssembly front end
tools/lipstool        inspect/convert data files (info, pics, map, render, biosfont)
tools/pilot           drive the original (Wine) and this engine headlessly; compare screenshots
re/                   format and engine notes, Ghidra scripts, Python validators
tests/                golden comparison and pilot scripts
```

## Running

```sh
# Desktop: point --data at the folder containing lips.exe, LIPS00.XRB, ...
cargo run --release -p lips-desktop -- --data /path/to/ExplodingLips

#   --classic            the original 320x200 screen shown at 4:3
#   --size WxH           window size (default 1280x960); F11 toggles fullscreen
#   --fullscreen         borderless fullscreen
#   --true-color         smooth RGB lighting (F9 toggles)
#   --fov DEG            vertical field of view (original 48.9)
#   --level LIPSnn       skip the menus
#   --soundfont F.sf2    play the MIDI music through a SoundFont
#   --software           present on the CPU (no GPU required)
#   --mod DIR            overlay replacement files (repeatable)
#   --host [--port N] [--ws-port N] / --join HOST[:PORT] / --name NAME   netplay
```

Settings, key bindings, saves and the quicksave live in `~/.config/exploding-lips/`
(`%APPDATA%` on Windows, `~/Library/Application Support` on macOS). On a Steam Deck the game
defaults to 1280x800 fullscreen.

**Web:** `./scripts/build-web.sh`, then `python3 -m http.server -d crates/lips-web/www 8080` and
open `http://localhost:8080`; choose your game folder. Add `?join=ws://HOST:PORT&name=you` to join
a desktop host started with `--host --ws-port PORT`.

### Controls

Original `keyboard.txt` bindings: arrows move/turn, Alt+arrows strafe, Tab jump, Caps Lock crouch,
W/X look up/down, S centre, Ctrl use tool, 0–9 select tool, F2 map, numpad `*`/`/` brightness,
Esc game menu, `~` console. Extras: A/D strafe, Shift run, Space jump, mouse look (click to capture),
F6 quicksave, F7 quickload, F10 options (incl. key rebinding). Gamepad: left stick move, right stick
look, A jump/select, X/RT use, LT run, Start menu, Select options.

## Tools

```sh
cargo run -p lipstool -- info  original/LIPS00.XRB
cargo run -p lipstool -- pics  original/LIPS00.XRB original/PALETTE.PAL out/
cargo run -p lipstool -- map   original/LIPS00.XRB lips00.png
cargo run -p lipstool -- render original LIPS00 view.png 1920x1080 90 0 truecolor
```

## Testing

```sh
./scripts/test-all.sh              # unit, data, gameplay and netplay tests, clippy, WASM, web smoke test
./scripts/test-all.sh --original   # plus the golden comparison against lips.exe under Wine
```

`--original` also needs the Windows/macOS targets for the cross checks when installed
(`rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin`).

The golden comparison (`tests/compare_original.py`) plays the original through its menus into
LIPS00 under Wine on a headless Xvfb display, reads the player's position from the game's memory
(via `tools/pilot/launcher/godd-launch.exe`, which also supplies the BIOS font Wine lacks), and
compares screenshots and movement rates with this engine. The launcher can also teleport the
player (`poke` in pilot scripts) and read bot state, which `tests/pilot/shots_to_kill.txt` uses
to measure how many shots a bot takes in the original. Requirements: Wine, Xvfb, xdotool,
ImageMagick, mingw-w64-gcc, Node (for the web test), a Chromium-based browser.

## Legal

Exploding Lips and the G.O.D.D. engine are © their respective owners. This project contains no
original code or assets and is intended for people who own the game.
