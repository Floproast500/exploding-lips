# Summoner (Volition, 2000) — revival plan

The first revival target for the engine. Summoner has no source port and no PC remaster. The
only re-release is the emulated PS2 version on PS4/PS5 (2024). The rights belong to THQ
Nordic, so the same rules as Exploding Lips apply:

- a clean-room reimplementation;
- the user's own Steam files (app 275570, v1.40);
- nothing from the game is ever committed or redistributed.

## What the installed game contains (surveyed 2026-10-13)

| Data | Count / size | Format | What it means for us |
|---|---|---|---|
| Archives | 11 `.vpp`, 1.5 GB | VPP v1 (`0x51890ACE`): 64-byte entries (60-byte name + size), 2 KB-aligned, uncompressed. [Public spec](https://www.redfactionwiki.com/wiki/.vpp); also used by Red Faction | A reader is a day's work; extraction already works |
| Textures | 8,357 `.tga`, 536 MB | Standard TGA | Feed packbuilder and Real-ESRGAN as they are |
| Animated bitmaps, fonts | 104 `.vbm`, 7 `.vf` | Volition formats shared with Red Faction ([format list](https://www.redfactionwiki.com/wiki/List_of_File_Formats_in_RF1)) | Small; community notes exist |
| Sound, music | 1,454 `.wav` | Standard WAV | engine-audio as is (positional, reverb, occlusion) |
| Game rules | 589 `.tbl` | **Plain-text tables** (`$Key: value`, FreeSpace style) | Most game behaviour is data, readable without disassembly |
| Levels | 60 `.s3d`, 342 MB | Binary, magic `S3DF` version `0x12` | The main reverse-engineering job |
| Level support | 60 `.pfg`, 37 `.vis`, 50 `.lcf` | Pathfinding graph, visibility, mesh-name lists | Navigation and culling |
| Models | 825 `.vim` | Binary meshes (version 6), with separate `_shadow` meshes | Skinned and static meshes |
| Animation | 671 `.mvf`, 156 `.svf` | `VMVF` v5 skeletal clips; `.svf` sets name their clips | Skeletal animation |
| Effects | 193 `.rfx` | Binary particle / effect definitions | Spells and impacts |
| Cutscenes | 94 `.csc` + 66 `*_cscript.tbl` | `CSCR` camera/actor tracks + text scripts | In-engine cutscenes |
| Movies | 3 `.bik` | Bink 1 (logos only) | Low priority: skip, or play through ffmpeg if present |

**The tables carry most of the game.** Text tables cover:

- **Characters:** hit points, AI aggressiveness, teamwork, field of view, detection range, turn
  rates, default weapons, special abilities.
- **Items, armour and materials.**
- **Experience curve.**
- **Quests and journal stages.**
- **Event flags** for each level.
- **Dialogue trees:** topics, NPC text, companion interjections, events fired from dialogue.
- **Level scripts:** navpoints, monsters with patrol action lists, doors with sounds and locks,
  click triggers.
- **Cutscene scripts, music and detail levels.**

What has to be recovered by observation is mostly the rules engine: combat timing, chain
attacks, the AI's use of the table parameters, and how events and actions behave.

## Engine features needed (beyond Exploding Lips)

1. **Static 3D worlds:** large triangle-mesh levels with materials, transparency and baked
   light, plus visibility from `.vis` and frustum/portal culling. This builds on the M9 BSP
   static-mesh path.
2. **Skinned characters:**
   - `AnimatedMesh` in engine-scene with GPU skinning in engine-gpu;
   - animation blending and layering (walk + attack), attachment points for weapons and shields;
   - blob or projected shadows from the `_shadow` meshes.
3. **3D collision and movement:** triangle-mesh collision, a capsule character controller and
   ground following. Pathfinding runs on the `.pfg` graph (A*, party formation).
4. **Third-person and party control:**
   - orbit camera with collision;
   - click-to-move and WASD/pad;
   - four-member party with selection and orders;
   - real-time with pause.
5. **Widget UI on the GPU UI list:** dialogue trees, inventory and paper doll, character
   sheet, journal, world map, shops, menus. This is the next step for engine-ui; generic
   widgets come from it.
6. **Data layer:**
   - a Volition `.tbl` parser with typed readers per table;
   - an event-flag store;
   - a quest/dialogue state machine;
   - our own save format.
7. **Effects:** a particle system driven by `.rfx` (billboards, trails, lights) and
   spell-light hookup to the existing dynamic lights.
8. **Cutscenes:** a track player for `.csc` camera and actor curves, triggered from
   `*_cscript.tbl`.
9. **Remaster layer (existing, extended to meshes):**
   - HD texture pack from the TGAs;
   - normal and specular maps;
   - modern lighting with shadows from the main light;
   - MSAA, FXAA and render scale;
   - widescreen and ultrawide, high-DPI UI;
   - controller and Steam Deck layouts, rebinding, rumble, gyro aim for spells.

## Milestones

Each milestone ends with tests and CI green, as in Exploding Lips.

| # | Milestone | Done when |
|---|---|---|
| S0 | **Setup:** `summoner-formats` and `summoner-game` crates, `sumtool` (ls / extract / tbl check). Golden harness: Summoner.exe under Wine + Xvfb, scripted with the pilot (D3D through WineD3D/DXVK), screenshots and memory probes like `godd-launch` | All 11 VPPs list and extract; all 589 tables parse with zero errors; the original reaches Lenele under the pilot |
| S1 | **Levels:** S3D reader and a level viewer module (like the BSP viewer): geometry, materials, TGA textures, baked light, visibility | All 60 levels load and render; camera at table navpoints matches the original's screenshots (RMSE threshold like the Lips golden test) |
| S2 | **Characters:** VIM meshes, SVF/MVF skeletons and animations, GPU skinning, attachments; model viewer | All 825 meshes load; every character plays its walk/attack/idle clips; poses match original screenshots |
| S3 | **World:** level scripts (navpoints, objects, doors, clicks), collision and controller, third-person camera, pathfinding on `.pfg` | Walk Wolong and Lenele; monsters patrol their action lists; doors open with their sounds |
| S4 | **Combat and RPG rules:** characters, items, armour, materials, experience, abilities, chain attacks, summons, AI parameters, party | Scripted fights reproduce the original's damage, hit counts and XP (measured under the pilot); party follows and fights |
| S5 | **Story:** event flags, dialogue trees, quests/journal, level transitions, world map, cutscenes | Prologue (Wolong to Lenele) playable from start to finish with all dialogue, quests and cutscenes |
| S6 | **Presentation:** full UI (inventory, character, journal, shops, options), audio (positional, music table), saves | Vertical slice playable without the original's UI; save/load round-trips |
| S7 | **Remaster:** HD pack from the TGAs, normal/spec, modern lighting with shadows, widescreen/4K, controller/Deck | 3440x1440 at 144+ fps on the RX 9070 XT; Deck at 60 |
| S8 | **Whole game:** remaining acts, bosses, endgame; multiplayer later (the original has co-op) | Pilot-driven playthrough checks per act; the user plays it start to finish |

**Vertical-slice rule:** S3–S6 target the prologue first (Wolong, Wolong caverns, Lenele). The
other 50+ levels then mostly exercise already-built systems.

## Clean-room process (unchanged from Exploding Lips)

- Formats come from data inspection and public documentation: the VPP spec and Red Faction
  community notes for VBM and VF.
- Behaviour comes from observing the original under Wine (pilot scripts, memory probes) and from
  the tables.
- Notes go to `re/summoner/*.md`. Code is written from those notes only.
- Any disassembly happens in a separate pass that produces notes, never code.
- Nothing from the game is committed: no extracted files, no packs, no tables.

## Risks

- **S3D complexity:** terrain, lightmaps and multiple planes (the tables mention `+Plane:`).
  Mitigation: start with geometry and textures and verify against screenshots; light comes
  second.
- **Animation compression in MVF:** mitigated by comparing poses against the original frame by
  frame.
- **Combat feel:** mitigated by measuring timings under the pilot, not guessing.
- **Original under Wine:** check D3D on Wine early (S0); fall back to the PS2 version's
  behaviour notes only if needed.
- **Scale:** a 40-hour RPG. The vertical slice keeps progress testable, and each later act is
  a checkpoint.

## Engine work that carries over

Skinned meshes, 3D collision, the widget UI, particles, cutscenes and the table parser are
exactly what the next candidates need:

- **Red Faction** (same studio and archive formats);
- **Anachronox** (Quake II BSP);
- **Slave Zero**.

Summoner turns the engine from 2.5D into a general 3D action-RPG engine.
