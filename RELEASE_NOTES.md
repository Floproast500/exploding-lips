# Big Lips 1.0.0

The first complete release of **Big Lips**, a source port of **Exploding Lips** (Galaxy Games /
G.O.D.D., 1999) on the **Floproast Kiln** engine.

All 18 levels play from the first room to the final boss. Play it as the original, or as a full
remaster at any resolution.

> **Bring your own data.** Big Lips contains no part of the original game. You need your own
> copy of Exploding Lips; Big Lips finds it on first run, or asks for the folder.

## Highlights

**A faithful port**
- A clean-room reimplementation of the G.O.D.D. engine in Rust. The movement, combat, scripts and level logic follow the original. They're tested against the original running side by side, with screenshot and physics comparisons.
- Classic mode reproduces the original's software renderer at 320×200, checked against screenshots of the original game.
- LIPS00's hidden Quake-style catacombs are fully playable.

**A full remaster, if you want one**
- **True 3D view:**
  - any resolution and aspect ratio, including 21:9 ultrawide;
  - real mouse look up and down;
  - MSAA, FXAA and render scaling.
- **HD textures:** AI-upscaled 4× from your own game files, with generated normal and specular maps.
- **Modern lighting:**
  - coloured dynamic lights from shots and explosions;
  - shadows from the brightest light;
  - bloom and filmic tonemapping.
- **3D sound:** positional sound with room reverb and occlusion. MIDI music always plays, thanks to a built-in SoundFont.

**Finding your way**
- **Route guide:**
  - a soft green beam over the next thing to do, and glowing dots along the way there, also shown on the automap;
  - every level has a route, cut down to just what the level needs, in an order where each door is open by the time you reach it;
  - steps you do early count; the guide always shows the first thing not yet done.
- **Simpler guides, if you prefer:** switches, exits and teleporters glow.
- **Automap:** explored areas, textured floors, zoom, rotation, markers and a compass.

**Play your way**
- **Keyboard and mouse:** WASD and 1:1 mouse look, or the original keyboard layout. Everything can be rebound.
- **Controllers:**
  - Xbox, PlayStation, Switch, Steam Deck and Steam Controller;
  - rumble, plus gyro aiming in the SDL3 build.
- **Saves and endings:** 8 save slots with thumbnails, and quicksave. Winning now shows an end screen with your totals.
- **Console:** `god`, `noclip` and `warp`.
- **Extras:** mods and texture packs, netplay (browsers can join), and the whole game in a web browser.

## Fixes to the original

These are on by default. Each one can be turned off ("Fix original game bugs" in Options) for
the game exactly as it shipped.

- Your shots hit an enemy anywhere on its sprite. In the original only a small box at its feet counted, so a level boss had to be hit near the bottom of its middle.
- Enemies only shoot at you, and only growl, when they can actually see you.
- Shots fly past a killed enemy's bone pile instead of bursting on it right in front of you.
- Several levels have floors, ceilings or walls missing in the original map data. They're closed up so the 3D view never shows sky through them.

## Fixed during testing

Every level was played through by test bots and by hand.
- The 3D view no longer shows sky through buildings when looking slightly up or down, near the screen edges, or right inside a door that's opening.
- Quickload is silent. It used to play every sound of the run at once.
- Routes no longer lead to doors that can't be opened yet, and no longer skip ahead when you do a step early.

## Requirements

- **Your game files:** a copy of Exploding Lips.
- **Desktop:** Linux, Windows, macOS or Steam Deck. A GPU with Vulkan, Metal or DirectX 12 for the 3D view; the software renderer runs anywhere.
- **Web:** a recent browser (WebGPU or WebGL2).

## Thanks

To the G.O.D.D. Game Development Group and Galaxy Games, who made Exploding Lips and its engine
in 1998–1999.
