# Big Lips

**Big Lips** is a source port of **Exploding Lips** (Galaxy Games / G.O.D.D., 1999), built on the
**Floproast Kiln** engine. It plays your copy of the original game on modern systems, either
faithfully or as a full remaster. Platforms: Linux, Windows, macOS, Steam Deck and the web.

It brings:

- AI-upscaled HD textures and modern lighting with shadows;
- ultrawide and high-DPI support, with an HD HUD;
- modern controls with gamepad, gyro and rumble;
- 3D sound, netplay and an automap.

Floproast Kiln is a clean-room Rust engine for reviving 90s–00s games that never got a port or
a remaster. Exploding Lips is its first game. It also runs Quake-family BSP maps, and the next
revival, Summoner, is already planned (see [Floproast Kiln](#floproast-kiln)).

> **Bring your own data.** Big Lips contains no part of the original game. You need your own
> copy of Exploding Lips; on first run Big Lips finds it (or asks for the folder) and builds
> the HD remaster from *your* files on your machine.

---

## Contents

- [Quick start](#quick-start)
- [What's new compared with the original](#whats-new-compared-with-the-original)
- [Faithful where it matters](#faithful-where-it-matters)
- [Playing](#playing): controls, controllers, options, HD textures, music, web, netplay
- [Floproast Kiln](#floproast-kiln): engine design, crates, other games
- [Tools](#tools)
- [Testing](#testing)
- [Building from source](#building-from-source)
- [Clean room and legal](#clean-room-and-legal)

---

## Quick start

```sh
git clone https://github.com/Floproast500/exploding-lips.git && cd exploding-lips
cargo build --release -p lips-desktop -p packbuilder --features lips-desktop/sdl
./target/release/biglips                      # finds your Exploding Lips folder, or asks for it
```

On first launch Big Lips does three things:

1. **Finds your game folder.** It looks for the folder with `LIPS00.XRB`: the configured one,
   next to the binary, `./original`, or `~/Games/Exploding Lips`. If none matches, it opens a
   folder picker. The choice is remembered, and `--data <folder>` overrides it.
2. **Builds the HD textures in the background.** `packbuilder` upscales every texture 4× with
   Real-ESRGAN when it is installed, and with a built-in pixel-art scaler otherwise. It adds
   normal and specular maps. The game switches to the pack when it is ready, after about a
   minute on a modern GPU. Turn this off with `set r_autopack 0`.
3. **Starts in the remastered mode:** GPU renderer, modern lighting and the modern HUD.
   `--classic` gives you the original 320×200 look.

`scripts/play.sh` builds and starts the remaster in one step. With a Steam Controller, add that
script to Steam as a non-Steam game (see [Controllers](#controllers)).

## What's new compared with the original

| | |
|---|---|
| **HD textures** | Every wall, floor, sprite and sky upscaled 4× by Real-ESRGAN. Tiling textures stay seamless and sprites keep clean edges. Generated normal and specular maps; mipmaps and anisotropic filtering. Texture coordinates match the original exactly. |
| **Lighting** | Light fitted to the original shade tables, in HDR. Coloured dynamic lights from shots, projectiles and explosions (clustered, 64 at a time), specular highlights, cube-map shadows from the brightest light, bloom, and soft or filmic tonemapping. |
| **Image quality** | Any resolution and aspect ratio (Hor+ widescreen, ultrawide), true 3D mouse look, 2×/4×/8× MSAA, FXAA, render scale from 0.25× to 2× (supersampling). 3440×1440 with everything on renders in about 0.7 ms on an RX 9070 XT. |
| **HUD** | Modern HUD: one compact, scalable cluster with Life, Armor, Money and bars for Magic and Experience. Score, time, tool, treasures, deaths and kills are on the automap. All HUD text, menus and the automap are drawn at display resolution with smooth edges. The original HUD is one option away. |
| **Way forward** | Optional guides: switches glow amber, the exit switch green, working teleporters blue, and the same markers appear on the automap. The original hides all of them; its switches look like plain walls. |
| **The catacombs** | LIPS00's hidden second world, a Quake-style BSP area reached from the start room, is fully playable. Its physics are the original's, and guide lights lead to the way out. |
| **Controls** | WASD and 1:1 mouse look by default, with the original keyboard layout as a preset. Every action can be rebound to keys, mouse buttons, wheel or pad buttons. Always-run, toggle-crouch, crosshairs. |
| **Controllers** | Xbox/PlayStation/Switch pads, Steam Deck and Steam Controller. Rumble on shots, hits and nearby blasts. Gyro aiming (SDL3 build). |
| **Audio** | Positional 3D sound, room reverb sized from the space around you, occlusion (sounds behind walls are muffled, shut doors block them), region background sounds, and streamed MIDI music with a built-in SoundFont so music always plays. |
| **Quality of life** | Automap with explored areas, textured floors, zoom, rotation and markers; 8 save slots with thumbnails; quicksave; console with completion; mods and texture packs; screenshots (F12). |
| **Netplay** | Host and join over UDP. Browsers join over WebSocket. Doors and switches are shared. |
| **Web** | The same game in the browser (WebGPU, WebGL2 or 2D canvas), with your game folder picked locally. |

## Faithful where it matters

Big Lips was written from the original's observed behaviour and file formats, and it is tested
against the original running under Wine:

- **Rendering.** The software renderer matches `lips.exe` within RMSE 0.04 on the 3D view and
  0.06 on the full screen. The GPU renderer matches the software renderer over 110 views on
  9 levels (worst case 0.033).
- **Movement.** Speeds, turning, jumping, falling and step heights are checked against the
  player position read from the original's memory.
- **Combat.** Shots-to-kill and fire rate are confirmed against `lips.exe`.
- **Scripts and effects.** Every shipped level loads, and its scripts, doors, lifts, switches,
  teleports and bots are covered by tests on the real data. That includes the original's
  quirks, kept on purpose.

With `--classic` you get the original 320×200 software look, shade tables and HUD.

## Playing

### Controls

The options (Video, Audio, Controls, HUD, Game, Accessibility and Key bindings) are in the menu
(Esc → Options, also on the title menu), or F10 at any time. Two binding presets:

- **Modern** (default):
  - WASD move, mouse look (click to capture);
  - Mouse1 or F use tool, wheel or Q/E switch tools, 0–9 pick a tool;
  - Space jump, Ctrl or C crouch, Shift run;
  - Tab map.
- **Classic:** the original `keyboard.txt`:
  - arrows move, Alt+arrows strafe, W/X look, S centre;
  - Tab jump, Caps Lock crouch, Ctrl use;
  - F2 map.

Both presets:

| Key | Action |
|---|---|
| Esc | menu |
| `~` | console |
| F6 / F7 | quicksave / quickload |
| F11 | fullscreen |
| F12 | screenshot (PNG in `screenshots/` beside your saves) |
| numpad `*` / `/` | brightness |

In the automap, +/- or the wheel zoom and M places a marker.

### Controllers

- **Layout:**
  - left stick moves, right stick looks;
  - A jumps or selects, X or RT uses the tool, LT runs;
  - LB/RB or D-pad left/right switch tools;
  - Start opens the menu, Select the options.
- **Rumble:** on by default (`joy_rumble`).
- **Gyro aiming:** needs the SDL3 build (`--features sdl`, SDL3 installed: `sdl3` on Arch), then
  `set joy_gyro 1`. `joy_gyro_sensitivity` sets the speed. It works on Steam Deck, Steam
  Controller, DualShock/DualSense and Switch Pro.
- **Controller check:** `biglips --pad-test 5` rumbles each pad and reports the gyro motion it
  reads.
- **Steam Controller and Steam Input:** start the game *from Steam*, by adding
  `scripts/play.sh` as a non-Steam game. Steam then gives the controller its gamepad layout.
  Under Steam the game uses X11, because Steam cannot see the focus of Wayland windows.

### Options and config

Options are console variables, saved to `config.cfg`:

| OS | Folder |
|---|---|
| Linux | `~/.config/exploding-lips/` |
| Windows | `%APPDATA%` |
| macOS | `~/Library/Application Support` |

The saves, quicksave, screenshots and the first-run HD pack live there too. The file holds plain
`set` / `bind` lines, the same commands the console accepts:

```
set r_fov 90        toggle cl_autorun        cvarlist
bind mouse:Right Jump        unbind key:KeyF
```

`--set NAME VALUE` changes a variable for one run only; settings changed in the game are saved.

### Fixes to the original game

Big Lips fixes a few bugs and softlocks that were always in Exploding Lips. They are on by
default; Options → Game → "Fix original game bugs" (`g_fixes 0`) gives the exact original
behaviour. Currently:

- **Doors with a switch on both faces** no longer shut when you touch the far face while walking
  through (the original closes them in front of, or on, you). They still close on their timers.
- **Trap pits** where landing sends you back to the start (LIPS00's sinking corridor) rise back
  once empty, so stepping onto the trigger and back out no longer locks you out of the exit.

Netplay players should use the same setting.

The console also has a `god` cheat: enemy shots and bots no longer send you back.
A selection:

| Variable | Meaning |
|---|---|
| `r_renderer` gpu / software, `r_classic` | renderer; original 320×200 screen |
| `r_lighting` modern / classic | HDR lighting or the original shade tables |
| `r_dynlights`, `r_shadows`, `r_bloom`, `r_tonemap`, `r_exposure` | lighting features |
| `r_msaa` off/2x/4x/8x, `r_fxaa`, `r_scale` | anti-aliasing and render scale |
| `r_texpack` folder, `r_autopack` | HD texture pack; build it on first run |
| `r_fov`, `r_freelook` | field of view; true 3D look |
| `r_stats` | frame and GPU time overlay |
| `hud_style` modern / classic, `hud_scale`, `hud_hd`, `hud_smooth` | HUD |
| `hud_guides` | glow on switches, teleporters and the exit |
| `crosshair`, `hud_fps`, `hud_highcontrast`, `msg_long` | HUD and accessibility |
| `m_sensitivity`, `m_yscale`, `m_invert`, `cl_autorun`, `cl_togglecrouch` | mouse and movement |
| `joy_sensitivity`, `joy_invert`, `joy_rumble`, `joy_gyro`, `joy_gyro_sensitivity` | controllers |
| `snd_master`, `snd_sfx`, `snd_music`, `snd_spatial`, `snd_reverb`, `snd_soundfont` | audio |

**Command line:**

| Option | Effect |
|---|---|
| `--data DIR` | game folder |
| `--classic` | the original 320×200 look |
| `--size WxH`, `--fullscreen` | window size, borderless fullscreen |
| `--level LIPSnn` | start directly in a level |
| `--software` | present on the CPU (no GPU) |
| `--soundfont F.sf2` | MIDI SoundFont |
| `--mod DIR` | overlay replacement files |
| `--host [--port N] [--ws-port N]` / `--join HOST[:PORT]` | netplay |
| `--screenshot-after S FILE` | take a screenshot after S seconds, then quit |
| `--pad-test [S]` | controller check |
| `--bsp PAK\|BSP MAP` | open a Quake-family map in the engine's viewer |

### HD texture packs

First run builds one automatically. To build or rebuild one by hand:

```sh
cargo run --release -p packbuilder -- /path/to/ExplodingLips packs/lips-hd
#   --upscaler esrgan|scale2x   --levels LIPS00,LIPS01   --normal-strength 2.0
./target/release/biglips --set r_texpack packs/lips-hd
```

Real-ESRGAN is found as follows:

- `$REALESRGAN`;
- `realesrgan-ncnn-vulkan` on the PATH;
- the portable release unpacked in `~/.local/share/exploding-lips/realesrgan/`.

Without it, the built-in scale2x keeps the pixel-art look. The whole game is 2,378 textures,
including the catacombs' BSP textures. Packs are built from your files and are for your own
use: don't share them.

### Music

The music is MIDI, streamed through a SoundFont. Big Lips uses, in order:

- `--soundfont`;
- the system's SoundFonts (`/usr/share/soundfonts`, for example `soundfont-fluid`);
- a small General MIDI SoundFont that the engine generates itself.

### Web

```sh
./scripts/build-web.sh
python3 -m http.server -d crates/lips-web/www 8080      # open http://localhost:8080
```

- **Starting:** choose your game folder; sound starts with your first click or key.
- **Renderer:** WebGPU, else WebGL2, else a 2D canvas. `?renderer=webgl|software` forces one.
- **Options:** `?set=r_lighting%20classic` sets any option. A `.sf2` in the game folder is used
  for music.
- **Settings and saves:** kept in browser storage.

### Netplay

- **Host:** `biglips --host`.
- **Join from another desktop:** `biglips --join HOST`.
- **Join from a browser:** the host starts with `--host --ws-port 27961`, and the browser opens
  `?join=ws://HOST:27961&name=you`.

Doors, switches and other players are shared.

## Floproast Kiln

Floproast Kiln is the engine under Big Lips. It is a set of game-agnostic Rust crates with games
plugged in as modules. Its purpose is to revive 90s–00s games that have no source port and no
remaster. Each one is reverse engineered clean-room from its own data, and then gets the same
modern features Big Lips has.

```
             ┌────────────── front ends ──────────────┐
             │ lips-desktop (winit, wgpu, kira, SDL3)  │  lips-web (WASM: WebGPU/WebGL2)
             └──────────────────┬─────────────────────┘
                     GameModule │  tick / view / audio / haptics
        ┌───────────────────────┼─────────────────────────┐
   godd-game (Exploding Lips)   quake-bsp (Quake, Hexen II maps)   … next: Summoner
        └───────────────────────┼─────────────────────────┘
   engine-core · engine-scene · engine-gpu · engine-audio  (shared by every game)
```

| Crate | What it does |
|---|---|
| `engine-core` | Frames, virtual file system with mod overlays, fixed-rate clock, typed console variables, bindings, config files, console, audio and haptic events, texture-pack format |
| `engine-scene` | Scene data for the renderer: sector outlines and triangulation with holes, scene frames, lights, cameras, the recorded UI draw list, the `GameModule` trait |
| `engine-gpu` | wgpu renderer: palette-indexed or HD textures, portal clip windows, clustered lights, shadow cubes, HDR post-processing (bloom, tonemap, FXAA), MSAA and render scale, GPU timing, HD UI, presenters |
| `engine-audio` | kira playback: positional sound, reverb, occlusion low-pass, buses, streamed MIDI through rustysynth, generated fallback SoundFont |
| `quake-bsp` | Quake-family BSP v29: PAK archives, textures, lightmaps, entities, clip-hull collision (Quake and Hexen II), a fly-through viewer module |
| `godd-formats` | Parsers for G.O.D.D. data (XRB levels, XPG pixels, PAL, PIC, INI scripts, GAM saves) |
| `godd-game` | The Exploding Lips module: world, software renderer, physics, script VM, effects, objects, combat, HUD, menus, automap, saves, guides, the BSP world |
| `godd-net` | Netplay protocol and transports (UDP, WebSocket) |
| `lips-desktop` | The `biglips` desktop front end |
| `lips-web` | The WebAssembly front end |
| `tools/packbuilder` | HD texture packs (Real-ESRGAN or scale2x, normal and specular maps) |
| `tools/lipstool` | Inspect, convert, render and benchmark |
| `tools/pilot` | Drive the original (under Wine) and Big Lips headlessly for comparisons |

**Other games today:** `biglips --bsp` opens Quake and Hexen II maps (from your own PAK files)
in the Kiln renderer as a fly-through with modern lighting:

```sh
./target/release/biglips --bsp "$HOME/.local/share/Steam/steamapps/common/Hexen 2/data1/pak0.pak" demo1
```

**Next:** the first new revival is **Summoner** (Volition, 2000). The plan is in
[`docs/summoner-plan.md`](docs/summoner-plan.md).

## Tools

```sh
cargo run -p lipstool -- info  original/LIPS00.XRB                      # level summary
cargo run -p lipstool -- pics  original/LIPS00.XRB original/PALETTE.PAL out/
cargo run -p lipstool -- map   original/LIPS00.XRB lips00.png            # top-down map
cargo run -p lipstool -- render original LIPS00 view.png 1920x1080 90 0 # software view
cargo run -p lipstool -- render-gpu original LIPS00 v.png 1920x1080 0 0 - 0 modern pack=packs/lips-hd
cargo run -p lipstool -- render-gpu original LIPS00 v.png 1280x720 0 0 - 0 modern tag:132   # the catacombs
cargo run -p lipstool -- dump-mesh original LIPS00 lips00.obj            # triangulated level
cargo run --release -p lipstool -- bench-gpu original LIPS00 3440x1440 0 0 - 0 modern pack=packs/lips-hd
```

`render-gpu` also takes:

- `set:NAME=VALUE`, to set any option;
- `boom`, to add an explosion;
- `map`, to open the automap;
- `tag:N`, to fire a script tag first.

## Testing

```sh
./scripts/test-all.sh              # unit, data, gameplay, netplay, GPU and audio tests, clippy, WASM, web smoke test
./scripts/test-all.sh --original   # plus the golden comparison against lips.exe under Wine
```

What the suite covers:

- **Data tests:** they use the original game files in `original/` and skip themselves without
  them.
- **GPU tests:** they run on any Vulkan, Metal or DX12 adapter, including lavapipe.
- **The golden comparison** (`tests/compare_original.py`):
  - it plays the original through its menus into LIPS00 on a headless Xvfb display;
  - it reads the player's position from the game's memory through a small launcher
    (`tools/pilot/launcher`);
  - it compares screenshots and movement with Big Lips.
- **Requirements:**
  - Wine, Xvfb, xdotool and ImageMagick;
  - mingw-w64-gcc;
  - Node and a Chromium-based browser, for the web test;
  - optionally the Windows and macOS Rust targets, for the cross checks.

CI runs on Linux (including the SDL3 build), Windows, macOS and WASM.

## Building from source

- **All platforms:** Rust (stable).
- **Linux:** `alsa-lib`, `libudev`, `dbus` and `pkg-config`; for the SDL3 build, `sdl3`.
- **Windows and macOS:** no extra packages.

| Command | Builds |
|---|---|
| `cargo build --release -p lips-desktop -p packbuilder` | the game and the pack builder |
| `cargo build --release -p lips-desktop --features sdl` | with SDL3 (gyro) |
| `cargo build --release -p lips-desktop --features sdl-build` | SDL3 compiled from source |
| `./scripts/build-web.sh` | the web build |

Ship `biglips` and `packbuilder` side by side: the game runs `packbuilder` on first run. Add
the Real-ESRGAN portable release next to them for AI upscaling.

## Clean room and legal

Exploding Lips and the G.O.D.D. engine are © their respective owners, who still hold the rights.
Big Lips and Floproast Kiln contain **no original code or assets**, and none are distributed:

- every player supplies their own copy;
- the HD textures are generated locally from that copy, and are for personal use only.

How the clean room works:

- The engine behaviour and file formats are documented as specifications in
  [`re/engine/`](re/engine/) and [`re/formats/`](re/formats/).
- Those specifications come from observing the original and from a separate analysis pass.
- The engine is written from the specifications, never from decompiler output.
