# Exploding Lips — engine reimplementation (and a retro engine)

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
| Renderer: portals, stacked (room-over-room) regions, floors/ceilings, sky, sprites with rotations, masked walls, original lighting | software renderer matches the original (golden test: RMSE 0.04 view / 0.06 full screen vs. `lips.exe`) |
| GPU renderer (wgpu): triangulated regions, portal visibility with exact screen windows, original shade tables or true colour, true-3D freelook | matches the software renderer (parity test: 110 views over 9 levels, worst RMSE 0.03); default on desktop (`r_renderer`) |
| HD texture packs (`tools/packbuilder`, `r_texpack`): 4x Real-ESRGAN upscales (seamless tiling, clean sprite alpha) with normal and specular maps, mipmaps, anisotropic filtering | texture coordinates verified unchanged (test: 4x copies render within RMSE 0.02 of the originals) |
| First-person weapon on the GPU (screen-space quad: HD packs apply) | parity test unchanged (worst RMSE 0.033) |
| Engine/game split: `GameModule` interface implemented by Exploding Lips and the BSP viewer; desktop `--bsp` runner | tested with LIPS00C.BSP, Hexen II demo1–3 and Quake E1M1 (when installed) |
| Modern lighting (`r_lighting modern`, default with the GPU renderer): smooth light fitted to the original shade tables, clustered coloured dynamic lights (16x9 tiles x 4 depth slices) with specular highlights on HD textures (shots, projectiles, explosions), cube-map shadows from the strongest light (`r_shadows`), HDR, 2x/4x/8x MSAA, FXAA, render scale 0.25-2x, bloom, soft/filmic tonemapping, exposure; `r_stats` shows frame and GPU time | 3440x1440 with the full HD pack, 4x MSAA and bloom: 0.6 ms GPU (timestamp queries) on an RX 9070 XT, 0.7 ms with an explosion's light and shadow; 8x MSAA 1.4 ms; 2x supersampling + 8x MSAA + FXAA 6.6 ms (`lipstool bench-gpu`); quality settings, clustering (every lit point is in a cluster listing its light; renders identical to unculled) and shadows (synthetic occluder) tested |
| Movement & collision (walk/run/turn/jump/fall/step, see-through floors) | constants from the original; golden-tested against memory probes of `lips.exe` |
| Script VM (`ONTAG`, tags, OS states with thresholds, `XM`/`XT`/`OTEXT`/`XCHANGE`/…) | complete for the shipped INIs, original quirks included |
| World effects: doors, lifts, jump pads, rotating rooms, ramps, rafts, switches, region triggers, scrolling/animated textures | implemented; data-driven tests on the shipped levels |
| Objects: bots (mover scripts), pickups, text, looping/positional sounds and sound sequences, wall/object morphs, flying, TV views, overlays, lightning, switches, timers, counters, teleports, level exits | implemented; data-driven tests |
| Combat: tools 0–9, projectiles, explosions (bomb multi-hit), bot attacks, first-person weapon | implemented; shots-to-kill and fire rate confirmed against `lips.exe` |
| HUD, status messages, menus (LIPSTART), character save slots (LIPSTART.GAM) | exact original layout |
| Audio: positional 3D sound (bots, doors, lifts, shots, explosions), room reverb sized from the surrounding space, occlusion (sounds more than a room away through open portals are low-passed and quieter; shut doors block them), effects/music/master volume, streamed MIDI with a built-in fallback SoundFont (`snd_spatial`, `snd_reverb`) | unit-tested (gain/pan, SoundFont parses and plays, streaming and stop) and data-tested (door sounds come from the door, shut doors cut rooms off) |
| Desktop: wgpu palette shader + CPU fallback, mouse, gamepad/Steam Deck, audio (WAV + MIDI via SoundFont) | Linux tested; Windows build cross-compiled and run under Wine; macOS type-checked; gamepad tested end to end with a virtual Xbox 360 pad (`tools/pilot/gamepad_pilot.py`, uinput); rumble on shots, damage and nearby blasts (data-tested); optional SDL3 backend with gyro aiming |
| Web (WASM): folder picker, GPU renderer on WebGPU or WebGL2 (software canvas fallback), positional audio with streamed music, options/bindings/saves in localStorage | working; headless-tested on WebGL2 and software (headless Chromium cannot show WebGPU canvases) |
| Netplay: UDP host/join, WebSocket for browsers, shared doors/switches, remote players | working |
| Modern extras: Hor+ widescreen, FOV, true colour, quicksave, key rebinding, high-contrast HUD, long messages | working |
| HD HUD, menus and automap: on the GPU renderer, text, boxes, menu images, map walls and markers are drawn at display resolution with smooth, anti-aliased edges (`hud_hd`, `hud_smooth`) | GPU-tested (headless draw of boxes, glyphs, images) and data-tested (HUD recorded for the GPU) |
| Automap (F2/Tab): explored regions only, textured floors, zoom (+/-, wheel), north-up or rotating, markers (M), other players | working |
| Save slots (F10 > Save/Load game): 8 slots with view thumbnails, stored in the config folder / browser storage | data-tested round trip |
| Console (`~`): config commands, Tab completion, history, scrollback, INI commands, `warp` | unit-tested |
| Mods & packs (F10): pick a texture pack, toggle mod folders from `mods/` (applied on restart), `--mod DIR` | working |

The engine is split into game-agnostic `engine-*` crates and game modules behind one interface
(`engine_scene::module::GameModule`). Exploding Lips is the first module; the second is a
**Quake-family BSP viewer** (Quake, Hexen II), the first step toward reviving more 90s–00s games:

```sh
cargo run --release -p lips-desktop -- --bsp "~/.local/share/Steam/steamapps/common/Hexen 2/data1/pak0.pak" demo1
cargo run --release -p lips-desktop -- --bsp original/LIPS00C.BSP lips00c palette=original/PALETTE.PAL
#   WASD/arrows fly, mouse look (click to capture), Space/Ctrl up/down, Shift faster, Esc releases/quits
cargo run --release -p lipstool -- render-bsp <PAK|BSP> <MAP> out.png [WxH] [palette=FILE]
```

Reverse-engineering notes live in [`re/`](re/): file formats in `re/formats/`, engine behaviour
in `re/engine/` (renderer, physics, script, movers, combat, HUD, object and world effects).
Engine code is written from those specs, not from decompiler output.

## Layout

```
crates/engine-core    game-agnostic core: frame, VFS, clock, cvars, bindings, config, audio requests
crates/engine-scene   scene data for GPU renderers: sector outlines, triangulation, per-frame scenes
crates/engine-audio   kira playback: positional sound, reverb send, buses, streamed MIDI, fallback SoundFont
crates/engine-gpu     wgpu renderer: palette-indexed world pass, portal clip windows, HDR/bloom, HD packs, UI
tools/packbuilder     build HD texture packs (Real-ESRGAN or scale2x, normal maps) from your own files
crates/quake-bsp      Quake-family BSP v29 module: PAK, palette/colormap, textures, lightmaps, fly viewer
crates/godd-formats   parsers for the data files (no platform dependencies)
crates/godd-game      engine core: world, renderer, physics, scripts, effects, objects, UI
crates/godd-net       netplay protocol, UDP/WebSocket server, client glue (feature "game")
crates/lips-desktop   native front end (winit + wgpu, kira audio, gilrs gamepads; SDL3 gamepads with --features sdl)
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
#   --true-color         smooth RGB lighting
#   --fov DEG            vertical field of view (original 48.9)
#   --set NAME VALUE     set any console variable (repeatable)
#   --level LIPSnn       skip the menus
#   --soundfont F.sf2    MIDI SoundFont (default: the first one found in /usr/share/soundfonts)
#   --software           present on the CPU (no GPU required)
#   --mod DIR            overlay replacement files (repeatable)
#   --host [--port N] [--ws-port N] / --join HOST[:PORT] / --name NAME   netplay
```

Options and bindings are console variables saved to `config.cfg` in `~/.config/exploding-lips/`
(`%APPDATA%` on Windows, `~/Library/Application Support` on macOS), next to the saves and the
quicksave. The file is plain `set` / `bind` lines, the same commands the console (`~`) accepts:
`set r_fov 90`, `toggle cl_autorun`, `cvarlist`, `bind mouse:Right Jump`, `unbind key:KeyF`. An old
`settings.ini` is imported once. On a Steam Deck the game defaults to 1280x800 fullscreen. The web
build keeps the same config in browser storage.

**HD texture packs** (modern lighting): build one from your own files, then point the game at it.
Packs are generated locally and must not be shared.

```sh
cargo run --release -p packbuilder -- /path/to/ExplodingLips packs/lips-hd
#   uses Real-ESRGAN when found ($REALESRGAN, realesrgan-ncnn-vulkan on the PATH, or the
#   portable release unpacked in ~/.local/share/exploding-lips/realesrgan/), else scale2x;
#   all 2374 Exploding Lips textures take about a minute on an RX 9070 XT
#   --upscaler esrgan|scale2x   --levels LIPS00,LIPS01   --normal-strength 2.0
cargo run --release -p lips-desktop -- --set r_texpack packs/lips-hd    # or `set r_texpack ...` in the console
```

Every image is upscaled 4x (tiling textures with wrap-around padding so edges stay seamless,
sprites with clean alpha) and gets a normal map; the renderer keeps the original texture
coordinates, filters with mipmaps and anisotropy, and lights the normal maps.

**Music** is MIDI, streamed through a SoundFont: `--soundfont`, then the system's
(`/usr/share/soundfonts`, e.g. `soundfont-fluid` on Arch), else a small built-in General MIDI
SoundFont the engine generates itself — so music always plays.

**Web:** `./scripts/build-web.sh`, then `python3 -m http.server -d crates/lips-web/www 8080` and
open `http://localhost:8080`; choose your game folder. The page uses WebGPU, else WebGL2, else a 2D
canvas (`?renderer=webgl|software` forces one); sound starts with your first click or key;
`?set=r_lighting%20classic` sets any option. A `.sf2` in the game folder is used for music. Add `?join=ws://HOST:PORT&name=you` to join
a desktop host started with `--host --ws-port PORT`.

### Controls

F10 opens the options (Video, Audio, Controls, HUD, Accessibility, Key bindings). The bindings page
rebinds any action to a key, mouse button, wheel direction or pad button and has two presets:

- **Modern** (default): WASD move, mouse look (click to capture), Mouse1/F use tool, wheel or Q/E
  switch tools, Space jump, Ctrl/C crouch, Shift run, Tab map, 0–9 tools.
- **Classic**: the original `keyboard.txt` (arrows, Alt+arrows strafe, Tab jump, Caps Lock crouch,
  W/X look, S centre, Ctrl use, F2 map).

Both: Esc menu, `~` console, F6 quicksave, F7 quickload, F11 fullscreen, F12 screenshot (PNG in
`screenshots/` beside your saves), numpad `*`/`/` brightness. `--set NAME VALUE` lasts for that
run only; settings changed in game are saved.
Options include always run, toggle crouch, mouse speed / vertical scale / invert, stick speed /
invert, crosshair, HUD scale and volume. Gamepad: left stick move, right stick look, A jump/select,
X/RT use, LT run, LB/RB or D-pad left/right switch tools, Start menu, Select options. Steam
Controller: launch through Steam with a Gamepad layout; on Wayland, start the game as an X11 window
(`env -u WAYLAND_DISPLAY`, as `scripts/steam-padtest.sh` does) so Steam can see it has focus.

Rumble is on by default (`joy_rumble`). For gyro aiming (Steam Deck, Steam Controller,
DualShock/DualSense, Switch Pro) build with the SDL3 backend and switch it on:
`cargo run --release -p lips-desktop --features sdl -- --set joy_gyro 1` (`joy_gyro_sensitivity`
scales it). The SDL3 build needs SDL3 installed (`sdl3` on Arch). `--pad-test [SECONDS]` checks
controllers in that build: it rumbles each one and reports the gyro motion it reads.

## Tools

```sh
cargo run -p lipstool -- info  original/LIPS00.XRB
cargo run -p lipstool -- pics  original/LIPS00.XRB original/PALETTE.PAL out/
cargo run -p lipstool -- map   original/LIPS00.XRB lips00.png
cargo run -p lipstool -- render original LIPS00 view.png 1920x1080 90 0 truecolor
cargo run -p lipstool -- render-gpu original LIPS00 view.png 1920x1080 90 0       # same view on the GPU
cargo run -p lipstool -- dump-mesh original LIPS00 lips00.obj                     # triangulated level
cargo run -p lipstool -- render-gpu original LIPS00 v.png 1920x1080 0 0 - 0 modern boom   # modern light + explosion
cargo run --release -p lipstool -- bench-gpu original LIPS00 3440x1440 0 0 - 0 modern     # frame times
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
