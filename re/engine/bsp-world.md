# BSP world ("poly" mode) of lips.exe: Quake BSP levels inside a G.O.D.D. game

Clean-room behaviour notes for the second world type in the engine: a Quake-1 `.BSP` map that the
player can be moved into from a normal sector ("orb", XRB) level and back. In the shipped game only
LIPS00 uses it (`LIPS00C.BSP`).

Sources: `re/ghidra/decomp.c`, `decomp_extra.c`, and objdump disassembly of `original/lips.exe`
wherever Ghidra lost x87 or stack arguments. Data was checked with `re/scripts/effect_dump.py` and
ad-hoc scripts that parse `LIPS00C.BSP` and walk its clip hull 1.

Confidence: **H** = read directly from the code path, **M** = code plus data inference, **L** =
guess. Notation is as in `object-effects.md`: `self`, `other`, `aux` = self+0x1b0,
`int0..int4` = aux+0x88..+0x98, `s9` = aux+0x7a, `S0..S4` = resolved sounds at aux+0x9c..+0xac,
`rt` = object +0x04, "edge test" = `rt & 0xf == 0`.

Units used in this document:
- **BSP coordinates are Quake map units**, stored as 32-bit floats. They have no scale and no
  offset relative to anything in the orb. The two worlds never share coordinates: every crossing
  goes through explicit numbers in aux ints or through an orb object's position.
- **BSP angles are float radians.**
- Orb angles are 2048 units per turn, as elsewhere.
- **BSP time ("tick")**: physics multiplies by `dt`, and **dt = 5 on every BSP frame**. See §3.1.
  This is a per-frame constant, not real time.

---------------------------------------------------------------------------------------------------

## 0. Summary

| item | finding | conf |
|---|---|---|
| World mode | global `DAT_00709404`: **1 = orb (sector) world, 2 = BSP world**. 3 exists but is unused (`FUN_004ce3bb`). The per-frame dispatcher `FUN_004bb183` runs exactly one world per frame | H |
| BSP load | `[startup] loadbspfilename=` in the **level's** INI (read on every level strap), or effect OLOADBSP. The load is deferred to the main-loop state machine (flag 0x40). Loading does **not** enter the BSP | H |
| Enter BSP | **ORPOLY** (tag/trigger/bump): sets the BSP player position to `int0,int1,int2`, heading `int3` (orb units), starts `S0` as background sound, requests mode 2 | H |
| Leave BSP | **ORORB**: an axis-aligned cube in BSP space (centre `int0..2`, half-size `int3`, default 20). When the BSP player is inside it, the orb player is teleported onto the ORORB object (same as OTELE's object destination) and mode 1 is requested | H |
| BSP triggers | **OPTAGSW** (and ORORB, which also registers here): the same cube; fires the object's own tag once per entry | H |
| ORJUMP, ORTELE | **no-ops** in this build | H |
| "DUNGEON6" | **not a destination.** It is ORPOLY's aux slot 0, the background sound played while in the BSP. The destination is the aux ints | H |
| BSP entities used | only `classname`, `origin`, `model`, `angle` keys. Classes: `info_player_start`, `func_door`, and the prefix `trigger*`. No `target`/`targetname` handling | H |
| Collision | Quake **clip hull 1** (the 32x32x56 player hull) of model 0 and non-trigger brush models, point traces, one slide plane, 18-unit step-up, 2-unit ground snap | H |
| Orb world while in BSP | **frozen**: no orb tick, no level change. Tags fired from the BSP still reach orb objects at once | H |

---------------------------------------------------------------------------------------------------

## 1. The six BSP effects

All six are registered in effect-table order (`effects.md`): OLOADBSP 190, ORJUMP 191, ORORB 192,
ORPOLY 193, OPTAGSW 194, ORTELE 195. Masks:
- `0x802c0` = TAG | TRIGGER | BUMP | 0x80000 (0x80000 is never handled).
- `0x20802c0` = the same plus CREATE (0x2000000, sent at XRB load before INIT).

The **player test** in these handlers is `FUN_00489f14(other)`. It is slightly stricter than the
`FUN_0048a05a` used by most O* effects: it accepts `other` if `other.flags & 0x40000` (the player
object), or `other`'s owner (+0x144) if `other.flags & 0x200000` and the owner has 0x40000.
Other "player-ish" bots do not count. **[H]**

Shared trigger gate ("GATE") of OLOADBSP, ORJUMP and ORPOLY: **[H]**
```
ev == 1 (INIT):  self.flags |= 0x200 (immovable); preload S0..S4 (FUN_0049e0bd)
ev == 0x40 (TAG):                         fire()
ev == 0x80 (TRIGGER) and rt & 0xf == 0:   fire()
ev == 0x200 (BUMP) and player_test(other) and rt & 0xf == 0:   fire()
```
(OLOADBSP additionally requires `aux != 0` for every path.)

### 1.1 OLOADBSP (190, 0x43fc43, mask 0x802c0): load a BSP. Confidence H. Not used in data.

```
fire():  if aux == 0: return
         bspPath = gameDir + s9 + ".BSP"      // DAT_00709088; gameDir = config+0x334; s9 = aux+0x7a
         bspPending (DAT_007093f4) = 1
         mainState (DAT_00515c7c) |= 0x40     // processed between frames, §2.2
```
- No extension test: `.BSP` is always appended. This differs from the INI key, which appends
  `.BSP` only when the name has no '.'.
- It does not switch worlds.

### 1.2 ORJUMP (191, 0x43fd81, mask 0x802c0): no effect. Confidence H. Not used in data.

GATE as above. `fire()` calls `RequestWorld(0)` (`FUN_004b9981(0)`), and that function only acts
on 1 and 2 (§2.3). So nothing happens. It was probably meant as "back to the orb" or a mode
toggle. Do not implement more than the INIT part.

### 1.3 ORPOLY (193, 0x4403e9, mask 0x802c0): enter the BSP world. Confidence H.

Fields:

| rec | off | type | meaning |
|---|---|---|---|
| aux | int0 +0x88 | i32 | BSP x (Quake units, plain integer, **not** 16.16) |
| aux | int1 +0x8c | i32 | BSP y |
| aux | int2 +0x90 | i32 | BSP z of the player **origin** (hull-1 origin, see §3.3). 0.1 is added |
| aux | int3 +0x94 | i32 | heading in **orb angle units** (2048 per turn) |
| aux | S0 +0x9c | Sound | background sound for the BSP stay (slot 0 name, e.g. "DUNGEON6") |
| self | +0x01 bit 1 | flag | flags \|= 0x200 on INIT |

```
fire():
  if aux != 0:
      SetBspPlayer(client 0, int0, int1, int2, int3)     // FUN_0048f3fd, below
      if S0 != 0:
          SetBackgroundSound(S0)                          // FUN_0049e701 on a fake region whose +0x100 = S0, §2.6
  RequestWorld(2)                                          // FUN_004b9981(2), §2.3
```

`SetBspPlayer(client, x, y, z, a)` (`FUN_0048f3fd`) writes to the client's BSP entity (§2.4):
```
e.x = float(x);  e.y = float(y);  e.z = float(z) + 0.1
e.yaw = a * (1/2048) * 6.283184 + 4.712388      // a·2π/2048 + 3π/2
if e.yaw >  6.283184: e.yaw -= 6.283184
if e.yaw < -6.283184: e.yaw += 6.283184
PlayerThink(e)                                   // FUN_00493d69, see note
```
- It does **not** reset velocity, the airborne flag, view pitch or the view height.
- **Note [H]:** the trailing `PlayerThink` runs while the world mode is still 1. Every entity
  accessor picks its array by mode: mode 2 uses the BSP entity array 0x74b70c, any other mode uses
  the orb "poly object" array 0x530004. So this call updates a slot of the wrong array and skips
  collision (collision runs only in mode 2). Treat it as a no-op. The BSP player settles on the
  first real BSP frame.
- The mode switch is deferred. The sound and the position change happen at once, inside the
  ActivateTag call.
- Heading convention, in BSP x/y axes: the BSP forward vector is `(−sin yaw, cos yaw)` (§3.2).
  With yaw = a·2π/2048 + 3π/2 this is `(cos θ, sin θ)` for θ = a·2π/2048. That is the orb's own
  convention (angle 0 = +x, counter-clockwise).
  **So int3 is simply the facing in the BSP's x/y plane, measured as in the orb.** int3 = 0 faces
  +X.

### 1.4 ORORB (192, 0x44054b, mask 0x20802c0): way back to the orb. Confidence H.

Fields:

| rec | off | type | meaning |
|---|---|---|---|
| aux | int0..int2 | i32 | BSP centre of the exit cube (cx, cy, cz) |
| aux | int3 | i32 | half-size r of the cube. **r < 1 means 20.0** |
| self | tag i16 +0x46 | | fired once per entry (via the tag-switch list, §1.5) |
| self | point/z/angle/... | | teleport template for the orb player (MoveObjToObject) |
| self | rt bit 0x08 | | "player inside" latch of the tag-switch list (§1.5) |

```
ev == 1:          flags |= 0x200; if aux: preload sounds
ev == 0x2000000:  (CREATE at XRB load; aux is dereferenced without a null check)
                  RegisterOrbExit(self, int0, int1, int2, int3)     // FUN_0048f4ef
all other events: ignored (TAG, TRIGGER and BUMP are in the mask but have no branch)
```

`RegisterOrbExit`:
- If the exit list count `DAT_005147a8` < 120: append {x, y, z, r'} with r' = (r < 1 ? 20.0 : r),
  plus a pointer to self in the parallel array.
- **Then always** call `RegisterTagSwitch(self, same args)` (§1.5).

So every ORORB is also a tag switch with the same cube.

The exit is tested each BSP frame by `FUN_0048fdf1`, after the tag-switch test (§2.5):
```
p = BSP player position (after this frame's physics)
for i in 0 .. exitCount-1:                       // first match wins
    if |p.x−cx| ≤ r and |p.y−cy| ≤ r and |p.z−cz| ≤ r:        (inclusive on both sides)
        MoveObjToObject(PLAYER, exitObj[i])      // FUN_0042ce2d, as OTELE (object-effects.md OTELE)
        if PLAYER.region (+0xfc) != 0: SetBackgroundSound(region)   // FUN_0049e701(region), §2.6
        RequestWorld(1)
        break
```
- MoveObjToObject copies the template's point x/y/z, z +0x2c, +0x30 and angle +0x24, plus the
  "keep unless −1.0" fields: height, radius, the +0x44 word, the speed blocks, and the sprite block
  if the template has one. It also copies the fly flag and relocates the region by point location.
  See OTELE in `object-effects.md`.
- For LIPS00, COFFIN1 has h = 48, r = 16 and no sprite. The player's own height and radius are
  therefore replaced by the template's whenever those are not −1.0.
  **Check this against your OTELE implementation; it is the same routine [H].**
- No sound or text is played by ORORB itself.

### 1.5 OPTAGSW (194, 0x440609, mask 0x20802c0): BSP-space tag trigger. Confidence H. Not used directly in data.

```
ev == 1:          flags |= 0x200; preload sounds if aux
ev == 0x2000000:  RegisterTagSwitch(self, int0, int1, int2, int3)   // FUN_0048f597
```

`RegisterTagSwitch`: if the count `DAT_005147ac` < 120, append {x, y, z, r' (r < 1 → 20.0)} and the
owner pointer. The entry's yaw field (+0x18 of the 0x34-byte entry) is never written, so it stays
0.

Per BSP frame (`FUN_0049046a`):
```
for i in 0 .. tagCount-1:
    o = owner[i]
    if p is outside the cube (strict: p < c−r or p > c+r on any axis):
        o.rt &= ~0x08
    else:
        o.rt &= 0x08                 // NB: clears every other rt bit of the owner (hidden 0x1000,
                                     //     inactive 0x2000, contact bits, effect-off 0x80000000)
        if o.rt == 0:                // newly entered
            o.rt |= 0x08
            ActivateTag(o.tag)       // i16 at +0x46; activator register not set (stale)
            return 1                 // stop scanning; later entries wait for the next frame
return 0
```
- The original clears the other rt bits as a side effect. That is harmless in LIPS00, because the
  only owner, COFFIN1, has rt = 0. A reimplementation should keep a private latch.
- **No reset of either list was found.**
  - The counters `DAT_005147a8` and `DAT_005147ac` are only ever incremented (every reference to
    them was checked in the disassembly).
  - Reloading LIPS00 therefore appends duplicate entries that point at freed objects. That is an
    original bug.
  - A reimplementation should rebuild both lists on every XRB load. **[M]**

**Marker entities [H code, M visual]:** when the BSP is loaded (`FUN_00490295`), one BSP entity of
type 3 ("VERTEX3D") is created per tag-switch entry. Fields:
- position = cube centre;
- yaw = 0·k + 3π/2;
- radius = r';
- health (+0x50) = 7;
- link = the orb object.

Behaviour:
- Rendering: the entity draws the linked orb object's picture at that spot.
- Each frame its think adds `entityIndex · π/128` to its yaw (index-dependent, which looks like a
  bug) and copies the orb heading back into the orb object's angle +0x24.
- It can be shot (§4.3):
  - each hit: −1 health and ActivateTag(951);
  - at 0 health: ActivateTag(952) and its velocity is cleared.
- It does **not** block movement.
- In LIPS00 the only marker (COFFIN1) has no sprite (+0x64 = −1). It is most likely invisible.
  Implementing markers is optional.

### 1.6 ORTELE (195, 0x4406c7, mask 0x802c0): no effect. Confidence H. Not used in data.

- INIT: flags |= 0x200, preload sounds.
- BUMP: calls `FUN_00489f14(other)` and discards the result.
- Nothing else. It looks like an unfinished "teleport inside the BSP".

---------------------------------------------------------------------------------------------------

## 2. Switching between the two worlds

### 2.1 Per-frame dispatch **[H]**

The main loop (frame cap as in `physics.md` §1) calls `FUN_004bb183` each frame:
- **mode 1** → `FUN_004bb442`: the orb frame. It covers input sampling, the orb tick
  `FUN_00472267` (all objects, timers, movers), the orb renderer, **and pending level changes
  (OSTRAP "strapping")**.
- **mode 2** → `FUN_0048faf9`: the BSP frame (§2.5).
  **None of the orb tick runs.** Orb objects, OTIMER, region effects, movers and level changes are
  all frozen while the player is in the BSP.
- Afterwards, in both modes: overlays, chat, the screen flip, and so on.

Things that **do** cross into the orb while in mode 2:
- **ActivateTag** calls made by BSP code. They synchronously deliver TAG (0x40) to orb
  objects, walls and regions, so handlers run immediately (sounds start, OSETF records its target,
  and so on). Anything that needs ticks waits until mode 1 resumes.
- **PLAYER-MOVED (0x10000)** sent to the orb player's effect (§3.6).
- The orb player's **angle** is rewritten every BSP frame from the BSP yaw:
  ```
  PLAYER.angle(+0x24) = round(yaw/6.283184·2048 − 1536) & 0x7ff
  PLAYER.point+0x34   = angle << 16
  ```
  The orb player's position is not touched.
- Loading another orb (`FUN_00471fe0`) forces mode 1.

### 2.2 Loading the BSP **[H]**

Two sources set `bspPending = 1` and the path `DAT_00709088`:
1. `[startup] loadbspfilename=` in an INI. `FUN_0046fc5b` reads `[startup]` from the startup INI
   at boot **and from the level's own INI on every level strap** (`FUN_0047167f` → `FUN_0046fc5b`
   with the orb INI buffer).
   - Path = gameDir + value. `.BSP` is appended if the value has no '.'.
   - Correction to `script.md`: this key is effectively per level. LIPS00.INI is the only INI
     that has it.
2. OLOADBSP.

After a level finishes loading (strap code, and the post-init at game start), the engine checks:
if bspPending, then mainState |= 0x40.

The state handler `FUN_004db8e8` runs between frames until mainState is 0 or 1, one flag per pass,
in this order: 1, 0x10, **0x40**, **0x80**, … For flag 0x40:
```
if bspPending:
    mode = 2 (temporarily)
    ok = LoadBsp(bspPath)            // FUN_0048fa06
    if ok: SetScreenPalette(...)     // FUN_004e9eca, DirectDraw palette
    bspPending = 0
    mode = 1
else:
    LoadBsp(NULL)                    // re-init only
mainState &= ~0x40
```

`LoadBsp(path)` (`FUN_0048fa06`):
1. If a BSP is already loaded (or initialised), unload it (`FUN_0048fc01`).
2. Reset the model caches.
3. If path is not empty, load it with `FUN_004ce9aa`.
   - On failure it logs "ERROR: can't find bsp" and clears both "loaded" flags.
   - On success it sets `bspLoaded (DAT_005147b0) = 1`.
4. `FUN_0048fcc0`:
   - load the palette `GODDPOLY.PAL` (or `QUAKE.PAL` when config byte +0xd bit 0x10 is clear;
     the logged run used GODDPOLY.PAL);
   - allocate the z-buffer;
   - set up the BSP camera (§5).
5. `bspReady (DAT_005147b4) = 1`.

`FUN_004ce9aa` (world load):
- Read the file. The version must be **29** (Quake 1).
- Map the lumps: entities, planes, textures, vertexes, visibility, nodes, texinfo, faces, lighting,
  clipnodes, leafs, marksurfaces, edges, surfedges, models.
- Split the entity lump into one string per `{`.
- For every entity whose classname starts with `"trigger"` (7-character compare), read
  `"model" "*N"` and set `modelDisabled[N] = 1`. Disabled brush models are skipped by collision,
  rendering and hitscan.
- **Widen every model's bbox by 32 units** on each side (mins −32, maxs +32). This widened bbox is
  used for the bbox prefilters in §3.3.
- `func_door` entities → type-4 entities (§4.1).
- Tag-switch markers → type-3 entities (§1.5).
- Create client 0, named "star0" (§2.4), and its player entity (type 0) at `info_player_start`.
- Create **one more** type-0 entity at `info_player_start`. It is a dummy body with no model in the
  shipped data, so it is invisible.

`info_player_start` placement (`FUN_00468797`):
- Take the first entity with that classname and parse `"origin" "%f %f %f"`.
- z += 0.01.
- Store the result also in `DAT_005d1608..1610`.
- If there is no such entity, use the average of all vertices of the faces in leaf 3.
- LIPS00C has one: (−448, 400, 136) → (−448, 400, 136.01). The game never uses it, because ORPOLY
  overrides the position.

**Load order in LIPS00 [H]:** XRB load (CREATE → ORORB registers its cubes) → INI `[startup]`
(bspPending) → … → between frames: LoadBsp. So the BSP is loaded right after the level, while the
player is still in the start room. The markers find the lists already filled.

### 2.3 RequestWorld (`FUN_004b9981`) and the 0x80 state **[H]**

```
RequestWorld(m):
  m == 1:                               mainState |= 0x80; stateArg = 1
  m == 2 and bspLoaded and bspReady:    mainState |= 0x80; stateArg = 2
  otherwise:                            nothing   (ORJUMP's 0 lands here; so does 2 without a BSP)
```

State 0x80 handler, run between frames:
```
mainState &= ~0x80
destroy every dynamic object (index ≥ DAT_00510784) whose flags have 0x10000
    (projectiles; plus its +0x18c child object if both carry the 0x80 markers)  // FUN_004a379c
clear the frame buffer to colour 0
if stateArg == 0: mode = 1
else:
    if mode == stateArg: return          // projectiles were still destroyed
    mode = stateArg
```

No other state changes:
- The orb player keeps its position, health and inventory. Its HUD values keep showing.
- The BSP player keeps whatever velocity and flags it had.

### 2.4 Clients and BSP entities (data model) **[H/M]**

**Client** (one per local/network player; only client 0 is used): 0x38 bytes at `DAT_005d149c`.

| off | type | meaning |
|---|---|---|
| +0x00 | char[16] | name ("star0") |
| +0x10 | i32 | index of its BSP entity |
| +0x14 | f32 | yaw offset (mouse yaw goes here only while dead) |
| +0x18 | f32 | view pitch, radians, clamped to ±1.570796 |
| +0x1c | f32 | roll (head-tracker device only) |
| +0x20 | f32 | accumulated mouse yaw (bookkeeping) |
| +0x24 | f32 | death tilt (set to 1.0472 on death; grows by 0.000192 per frame while the death anim plays) |
| +0x28 | f32 | **view height offset** added to the camera z. Starts at 0; crouch −20/+20 (§3.5) |
| +0x2c | u16 | input bits for the next frame (§3.5) |
| +0x30 | i32 | dead flag |

**Entity** (0x68 bytes, array at `0x74b70c`, count `DAT_00765740`): the fields the gameplay
uses.

| off | type | meaning |
|---|---|---|
| +0x00 | i32 | type: 0 GODDSTRT (player), 1 GODDOPNT, 2 S_EXPLOD (shot impact), 3 VERTEX3D (tag-switch marker), 4 func_door, 5..8 unused. Each type has callbacks {init, think, control, damage, touch, die} |
| +0x04/08/0c | f32 | x, y, z (BSP units; for the player, the hull-1 origin) |
| +0x1c | f32 | yaw, radians, wrapped into (−2π, 2π) |
| +0x20 | i32 | animation frame (player) / brush model number (door) |
| +0x24 | f32 | radius (markers) |
| +0x30 | ptr | linked orb object (markers) |
| +0x34/38/3c | f32 | velocity **in the entity's local frame**: x = right, y = forward, z = up. Units per tick |
| +0x40 | f32 | yaw rate, rad per tick |
| +0x48 | i32 | **airborne** flag (1 = in the air) |
| +0x50 | i32 | health (player 10, marker 7) / door direction code |

Entities are removed by swap-with-last. The arrays are rebuilt from scratch on each BSP load.

### 2.5 The BSP frame (`FUN_0048faf9`) **[H]**

```
FUN_004905d6 (simulate + draw):
  for each client:                     ApplyInput(client)         // FUN_0048f679, §3.5
  client0.inputBits = 0
  EntityTick()                         // FUN_004ceadf: clock += 5, dt = 5; run every entity's
                                       //   think; remove those whose think returns 1
  clear frame buffer to colour 0xFF; clear z-buffer
  draw BSP view from client 0          // FUN_004cecd8, §5
  if !client0.dead: draw first-person model "V_SHOT" (no such model ships, so nothing is drawn)
  draw the orb tool sprite (current tool, bottom centre), exactly as the orb's FUN_00456e36 does (combat.md §5)
  if DAT_00510df0 (HNH/CTF/...): status HUD FUN_004c9d43          (hud.md)
  overlay text pass FUN_004aae49(0): OTEXT texts, messages, ...   (hud.md)
  if TagSwitchScan():                  (fires at most one tag, §1.5)
  if OrbExitScan(): RequestWorld(1)    (§1.4)   -- both run every frame, in this order
MouseInput()                           // FUN_00490bac, §3.5
HeadTracker()                          // FUN_00490c9d, a VR device; ignore
KeyboardInput()                        // FUN_0049075c, §3.5  (sets bits for the next frame)
sync orb PLAYER angle from BSP yaw     // §2.1
```

So **the input sampled at frame N is applied at frame N+1**.

### 2.6 Background sound switch (`FUN_0049e701`, "CheckRegionSound") **[H]**

The same routine the orb uses when the player changes region. Region +0x100 is a Sound pointer.
ORPOLY passes a fake region whose +0x100 = S0.
```
if sound enabled and region.snd != 0 and region.snd.name != currentBackgroundName:
    if name == "BACKSND": stop the current background; restart the level default background (DAT_00567724)
    elif name == "BSNDOFF": stop the current background; background off
    else: stop the current background; play region.snd as the new background
          (FUN_0049e5cf; remembers the sample name in DAT_0051550c)
```

In LIPS00:
- entering the BSP starts **DUNGEON6** (`snd/dungeon6.wav`);
- returning puts the player in region 26 "COFFIN1", whose region sound is Sound #25 **TOCATTA**.

---------------------------------------------------------------------------------------------------

## 3. Player movement and collision in the BSP

### 3.1 Time step **[H]**

- `FUN_00495485` returns a private clock that **adds exactly 5 on every call**. The PIT-timer path
  (`DAT_00514a8c`) is a DOS leftover and is always off on Win32.
- `EntityTick` computes `dt = clock − lastClock`. That is **dt = 5 on every BSP frame**, and 0 on
  the very first call after program start.
  - The orb frame's poly-object tick `FUN_004cec21` uses the same clock, so dt stays 5.
- **BSP physics is therefore frame-locked: every number below is "per frame".**
  - The original frame rate is the main loop's cap: a frame only when more than 33 ms have passed
    since the last one, i.e. ≤ ~30 fps; see `physics.md` §1.
  - To match original feel, run BSP physics at the original frame cadence, or scale by
    (your_dt / original_frame_time).

### 3.2 Player think (type 0, `FUN_00493d69`), once per frame **[H]**

```
old = e.pos
Friction(e)        // FUN_004cf559
Gravity(e)         // FUN_004cf27d
Integrate(e)       // FUN_004cf13f
if collisionEnabled (DAT_00514800 = 1 in the data) and mode == 2:
    target = e.pos; e.pos = old
    airborne = ClipMove(world, e, pos=e.pos, target)     // FUN_00410313, §3.3; writes e.pos
    LandingLogic(airborne)                               // §3.4
Footsteps()                                              // §3.6
Animation()                                              // stand / rockrun / rockatt / deatha frame names (Quake player.mdl names); visual only
```

Constants:

| name | value | where |
|---|---|---|
| ACCEL factor | 0.4 × 1.5 = 0.6 per tick | `DAT_005163d8` (f32 0.4) × `0x525398` (f64 1.5) |
| MAXSPEED (per axis) | 1.5 units/tick | `DAT_005163dc` |
| FRICTION factor | 1.5 × 0.2 = 0.3 per tick | `DAT_005163dc × DAT_005163d4` |
| JUMP speed | 3.5 units/tick | `DAT_005163cc` |
| GRAVITY factor | 3.5 × 0.02 = 0.07 per tick | `DAT_005163cc × DAT_005163d0` |
| TURN rate | 0.0196350 × 2 = 0.0392699 rad/tick (π/80) | `DAT_005163c8` × `0x5253a0` (2.0) |
| TURN decay | 0.0196350 rad/tick **per frame**, not dt-scaled | `DAT_005163c8` |

Each step, in order:

- **Input (`FUN_004cf477`, from ApplyInput):** for a move axis k ∈ {0 = right, 1 = forward},
  `v[k] += ±(dt·0.6)`; that is ±3.0 per frame. For turning, the yaw rate is set (not added) to
  ±0.0392699.
- **Friction:** for k in {x, y} only:
  - if v > 0: v −= dt·0.3 (= 1.5 per frame); if v > 1.5 then v = 1.5; if v < 0 then v = 0;
  - mirrored for v < 0.
  - The yaw rate moves toward 0 by 0.0196350 and is clamped at 0.
- **Gravity:** if airborne, `vz −= dt·0.07` (= 0.35 per frame). There is no terminal velocity.
- **Integrate:** if any velocity is non-zero:
  ```
  pos += R(yaw)·v·dt
  R(yaw) = [[cos, −sin, 0], [sin, cos, 0], [0, 0, 1]]
  ```
  So world dx = (c·vx − s·vy)·dt and dy = (s·vx + c·vy)·dt. Then `yaw += yawRate·dt`, wrapped
  into (−2π, 2π).
- **Jump** (`FUN_004cf300`, from ApplyInput): if not airborne, vz = 3.5. It is not dt-scaled.
  Holding jump re-jumps on every landing.

Resulting per-frame behaviour (dt = 5):

| behaviour | value |
|---|---|
| walking | instant start and stop. +3.0 − 1.5 = 1.5 per frame, so v = 1.5 u/tick on the first frame and **7.5 units per frame**; it drops to 0 on the first frame without input. Diagonal: each axis is clamped separately, so 7.5·√2 |
| turning | the yaw rate after friction is 0.019635, giving **0.0981748 rad = π/32 per frame** (= 32 orb units per frame). It stops immediately on release |
| jump | the first frame rises 17.5. Each airborne frame then does vz −= 0.35 before moving vz·5. **Apex ≈ 96.25 units** after 11 frames (17.5 + 5·0.35·45). The rooms in LIPS00C are often only 56 units of hull space high, so jumps hit ceilings |

### 3.3 ClipMove (`FUN_00410313`), the collision **[H]**

All traces use **Quake clip hull 1**:
- it starts at `model.headnode[1]` (model +0x28);
- clipnodes are 8 bytes {planenum i32, children i16[2]}; planes are 20 bytes;
- contents −2 = solid, any other negative = empty.

The player is treated as a point in hull-1 space. That is, a 32×32×56 box whose origin is 24 above
its feet, as compiled by qbsp. The traces are the standard iterative "SV_RecursiveHullCheck"
style: a stack of split points, with the 'front' child when the distance is ≥ 0. Every model's
origin is subtracted first. Models are scanned in this order: model 0 (world), then each brush
model k ≥ 1 that is not disabled and whose **widened** bbox passes a box test.

Helpers:
- `UpDist(model, p)` (`FUN_0040fe0c`): trace from p up to p + (0, 0, 100). Return the z distance
  from p to the **first non-solid point** on that segment. Return 0 if p itself is empty, and 100
  if it is solid all the way.
- `Trace(model, a, b)` (`FUN_00410069`): if the segment hits solid, return the impact point
  (where it enters the solid) and the clipnode/side of the plane crossed.
- `InEmpty(model, p)` (`FUN_0040fd16`): point contents ≠ solid.

```
ClipMove(pos, target):                                   // pos = start, target = wanted end
  // 1. ground probe and step-up
  target.z −= 2
  h = UpDist(world, target); for each model k≥1 enabled with target inside its widened bbox: h = max(h, UpDist(k, target))
  target.z += 2
  airborne = (h == 0)                    // nothing solid within 2 units below the target
  if 2 < h < 20:                         // target is inside the floor by h−2 (≤ 18): step up
      pos.z    += h − 2 + 0.01
      target.z += h − 2 + 0.01
  // (0 < h ≤ 2: standing on the floor; h ≥ 20: too high to step, left to the trace)

  // 2. sweep pos → end against every model
  end = target; hit = false
  for each model k (world, then enabled brush models whose widened bbox overlaps bbox(pos, end)):
      Touch(entityOfModel[k], player)    // brush-model owner callback (func_door), §4.1.
                                         // model 0 has no owner (−1): treat as no-op
      if Trace(k, pos, end) hits: end = impact; plane = that clipnode's plane; side = start side; hit = true
  if !hit:                         pos = target; return airborne
  if plane index is invalid:       pos = target; return airborne     (original: clipnode < 0 or ≥ numclipnodes)

  // 3. one slide along the hit plane
  v = target − pos
  pos = end − 0.01·v                                 // back off slightly from the impact
  d = dot(n, target) − plane.dist + (side ? +0.01 : −0.01)
  target = target − d·n                              // project the target onto the plane, 0.01 on our side
  repeat the ground probe of step 1 on the new target (pos is NOT raised this time):
      airborne = (h == 0); if 2 < h < 20: target.z += h − 2 + 0.01
  if target is in solid in any enabled model whose widened bbox contains it:
      keep pos (the backed-off impact point)
  else:
      pos = target
  return airborne
```

Consequences:
- Max step height is just under **18 units**. Ground snap is 2 units. Anything that drops more than
  2 below makes the player airborne, and gravity starts next frame.
- Only **one** slide plane per frame. The slide target is not re-traced, only point-tested, so
  inside corners stop the player rather than sliding twice.
- Ceilings and floors are handled by the same 3D sweep, because vertical velocity is part of the
  segment.
- **Entities never block the player.**

### 3.4 Landing, falling and the related tags **[H]**

After ClipMove (`e.airborne` is the value from the previous frame):
```
if airborne_now:
    if !e.airborne: fallStartZ = e.z (after this frame's move); fallTracking = 1
    e.airborne = 1
else if e.airborne:                        // just landed
    e.airborne = 0; e.vz = 0
    if jumpFlag (DAT_00510ce0, set by the jump input):
        jumpFlag = 0; ActivateTag(997)     // LIPS00: OSOUND #311 GRUNT01
    else:
        if fallTracking and fallStartZ − e.z > 100: Hurt()
        fallTracking = 0
```

`Hurt()` (`FUN_0045bb59`):
- If the player's controller (+0x1b8) has an object at +0x248 with an aux, play that aux's S2
  (+0xa4).
- Otherwise ActivateTag(997).
- **No health is removed** from the orb player.

### 3.5 Input mapping in BSP mode **[H]**

Keyboard (`FUN_0049075c`) reads the engine's "key held" table. In BSP mode, keys 0–9 still go to
the normal command dispatcher (tool selection); every other key only sets the held table. Game-key
indices are the remapped VK codes:

| key (default) | bit / action |
|---|---|
| Up | 0x01 forward |
| Down | 0x02 back |
| Left | without strafe: 0x10 turn left (yaw +); with Alt or the strafe key (game key 0x12) held: 0x04 strafe left |
| Right | 0x20 turn right / 0x08 strafe right |
| Ctrl (0x11) | 0x80 fire, and ActivateTag(943) **every frame held** |
| Tab (0x09) | 0x40 jump. ActivateTag(940) once per press; the latch resets when Tab is released or on landing. Sets jumpFlag |
| CapsLock (0x14) | press: bit 0x100 (view height −20) + ActivateTag(941). Release, seen by GetAsyncKeyState(CapsLock): bit 0x200 (view height +20). The hull does not change |

Mouse (`FUN_00490bac`):
- dx and dy are the cursor offset from the window centre (the cursor is re-centred each frame),
  averaged over the last 4 samples.
- Left button → 0x80 fire + tag 943.
- Right button → 0x01 forward, or 0x02 back if Space is held.
- Middle button → jump, like Tab.
- `yaw −= dx · π/512` (applied directly to the entity, wrapped). While dead, the change goes to the
  client's yaw offset instead.
- `pitch += dy · π/512`, clamped to ±π/2. The sign on screen is not verified **[L]**.

ApplyInput (`FUN_0048f679`), at the start of the next frame:
- 0x01 / 0x02 / 0x04 / 0x08 → velocity accel (§3.2).
- 0x10 / 0x20 → yaw rate.
- 0x40 → jump.
- 0x80 → if dead: respawn. That clears the client's angles and dead flag and creates a new player
  entity at info_player_start. Otherwise: **attack** (§4.3).
- 0x100 / 0x200 → view height ∓20.

### 3.6 Footsteps / PLAYER-MOVED **[H]**

Each think:
- If vx = vy = 0: once, remember stepPos = pos.
- Else, if not airborne: if |x − stepX| > 100 or |y − stepY| > 100:
  - send event 0x10000 to the **orb** player's effect (OPLAY). OPLAY plays the player aux S0 if
    rt bit 0x02000000, then ActivateTag(996);
  - stepPos = pos.

---------------------------------------------------------------------------------------------------

## 4. Interaction inside the BSP

### 4.1 BSP entity classes the engine reads **[H]**

These are the only strings the engine looks up in the entity lump: `classname`, `origin`, `model`,
`angle`.

| classname | handling |
|---|---|
| `info_player_start` | first one: spawn and respawn point (origin z + 0.01) |
| `trigger*` (any classname starting with "trigger") | its brush model `*N` is disabled: not solid, not drawn, not hit by shots. Nothing fires |
| `func_door` | type-4 entity for brush model `*N` (1 ≤ N < nummodels). Direction code from `angle`: 0 → +X, 90 → +Y, 180 → −X, 270 → −Y, −1 → up, −2 → down, anything else → +X. **Every touch** (each frame the player's sweep overlaps its widened bbox) and **every shot** moves the model origin by 10 units in that direction, permanently. There is no return, no wait, no speed and no targetname |
| everything else (`worldspawn`, `light*`, `ambient_*`, `info_tf*`, `info_intermission`, …) | ignored. Lights are already baked into the lightmaps |

There is **no** target/targetname, trigger_teleport, button, message or sound support. **All
scripted interaction in the BSP world goes through orb objects:** OPTAGSW/ORORB cubes (§1.4–1.5)
and the tags the BSP code fires (table below).

### 4.2 Tags fired by BSP mode (orb listeners react immediately) **[H]**

| tag | when |
|---|---|
| owner tag (+0x46) | OPTAGSW/ORORB cube entered (once per entry) |
| 940 | jump pressed (once per press) |
| 941 | crouch pressed |
| 943 | fire held (every frame) |
| 950 | attack animation started |
| 951 | a BSP entity took damage and survived (player or marker) |
| 952 | a BSP entity was killed (player death or marker) |
| 960 | any entity hit by a shot |
| 996 | footstep (from OPLAY, via 0x10000) |
| 997 | landed after a jump, or a hard landing when there is no hurt sound |

LIPS00 listens only to 997 (OSOUND #311 "GRUNT01") among these. Tags 132 and 134 are covered in
§6.

### 4.3 Attack (type-0 control `FUN_004940b6` → `FUN_0049c010`) **[M]**

- Only while the player entity's health > 0.
- If the attack animation is not already playing: ActivateTag(950), start the "rockatt" anim, and
  fire once. Then fire once more. So the first frame fires twice, and later held frames fire once
  per frame.
- Fire = a hitscan:
  - origin = player pos + (0, 0, 16);
  - direction = (0, 1, 0) rotated by yaw (+ yaw offset) and by pitch;
  - traced against the world hull, enabled brush models and entities (`FUN_00410d02`).
- An entity hit gets damage 1 (its type's damage callback) and ActivateTag(960).
- A hit door model moves 10 units (§4.1).
- An S_EXPLOD (type 2) entity is spawned at the impact point.
- No orb objects are hurt.

The player entity has 10 health:
- damage: health −= amount, ActivateTag(951);
- at 0: tag 952, death anim, client dead = 1, tilt 1.0472, view height −12.

Nothing in LIPS00C can damage the player, so this path is unreachable in the shipped game.

---------------------------------------------------------------------------------------------------

## 5. Rendering points that affect correctness (brief) **[M]**

- **Camera:**
  - position = player entity origin + (0, 0, client view height). The view height starts at 0, so
    the eye is at the hull-1 origin, **24 units above the feet** (Quake used +22 above the origin).
  - Rotation: yaw, roll (client +0x1c/death tilt), and pitch (`FUN_004cf085`).
- **Projection** (`FUN_004091dc` called with 1.5707964 = π/2):
  - centre = (w/2 − 0.5, h/2 − 0.5);
  - focal = (w/2) · 0.75 / tan(π/4) = **0.375·w**;
  - aspect term (+0x50) = h / (0.75·w), which is 1.0 at 640×480.
  - This gives **horizontal FOV ≈ 106.3°** and vertical 90° on 4:3. **[M]**: the use of +0x50 in
    the span code was not traced.
- **The 1.2361 constant is not BSP.** It is `DAT_00567124`, set in `FUN_004ba977` and used by the
  orb floor/ceiling span setup `FUN_004187f8`. "FUN_00418a72" is an address inside that function.
  `renderer.md` line 9 should be read that way.
- **Visibility:**
  - find the leaf by walking hull-0 nodes (`FUN_004078a6`), use the PVS, draw the world;
  - then enabled brush models 1..n−1 (trigger models are skipped);
  - then entities: type 1 = MDL model by type name, type 2 = sprite, type 3 = linked orb object
    picture.
  - No MDL files ship, so only the world (and possibly markers) is visible.
- **No sky handling.** LIPS00C has 5 textures (Cmnt4s, Wood7, Gbmetl08, Brick1, Tech01) and no
  sky. The frame is cleared to palette index **255** before the BSP draw.
- **Palette:** `GODDPOLY.PAL` (8962 bytes) is used:
  - 768 bytes of 6-bit RGB;
  - u8 table count (1);
  - u8 log2 light levels (5);
  - one colormap of 32 × 256 bytes.
  - On load the screen palette is switched (`FUN_004e9eca`, DirectDraw 8-bit modes). In a
    true-colour reimplementation, render BSP textures through this palette and colormap.
- **HUD in BSP mode:** the same tool sprite at the bottom centre, the same status HUD and the same
  overlay text pass as the orb view (§2.5; `hud.md` line 59, `combat.md` §5). There is no
  crosshair or automap.

---------------------------------------------------------------------------------------------------

## 6. LIPS00 data and the end-to-end sequence

### 6.1 Objects with effects 190–195 (all levels) **[H]**

Only two records exist in the whole game, both in LIPS00:

| obj | name | effect | tag | flags | orb position (x, y, z) | region | angle | aux |
|---|---|---|---|---|---|---|---|---|
| #101 | "" | ORPOLY (193) | 132 | 0x01000230 | (15232, 16896, 0) | −1 (none, so it can only fire by tag) | 512 | ints [2640, 832, 448, 0, 0]; s0 = S0 = "DUNGEON6" |
| #672 | COFFIN1 | ORORB (192) | 134 | 0x01000230 | (17272, 16784, −512) | 26 "COFFIN1" | 1024 | ints [−2384, −1120, 448, 32, 0]; h = 48, r = 16, no sprite |

Related records:

| record | what |
|---|---|
| wall 2493 | WTAGSW, tag 132, one-sided, region 642, from (11648, 16768) to (11776, 16768): the start-room switch |
| obj #681 | OSOUND tag 132: S0 "DOOR5CLS" |
| obj #14 COFFIN | OSETF tag 134: region "COFFIN1" floor → 0 at speed 100; sounds AMBSFX06/AMBSFX02/AMBSFX06 |
| wall 127 | WTAGSW tag 134: two-sided passable line in region 26 (17088, 16768)–(17088, 16800). It fires 134 again when crossed |
| region 26 "COFFIN1" | floor −512, ceiling 384, region sound TOCATTA |
| player start | (11008, 15616, 160), region 640, angle 512 |

### 6.2 LIPS00C.BSP summary **[H]**

- Version 29.
- **One model only (worldspawn)**: bbox (−3423, −1503, −127)…(3679, 1247, 543), 1902 faces,
  5 textures.
- The entity lump is left over from a TF map ("GoreBag's Sniper Jungle").
- **No brush entities, no triggers, no doors.** Classname counts:
  - light 144;
  - light_torch_small_walltorch 1;
  - ambient_swamp1 8;
  - info_tfgoal 4;
  - info_intermission 2;
  - info_tfdetect 1;
  - worldspawn 1;
  - info_player_start 1 at (−448, 400, 136).
- No targetnames are referenced by any orb object.
- The two bases are symmetric. ORPOLY's spawn (2640, 832) and ORORB's cube (−2384, −1120) are
  the same upstairs room in opposite bases. Both have a hull-1 floor at origin z = 424 (feet at
  400) and a hull-1 ceiling at 480.

### 6.3 What the player experiences (expected original behaviour) **[H for the mechanics, M for the walk route]**

1. **Load LIPS00.**
   - XRB CREATE registers COFFIN1: exit cube and tag cube, centre (−2384, −1120, 448), half-size 32.
   - The level INI sets bspPending; LoadBsp(LIPS00C.BSP) runs before the first frame. The player
     entity is at (−448, 400, 136.01), unused.
   - The background sound is the level's normal one.
2. **The player touches wall 2493** (sealed start room) → ActivateTag(132), in object order:
   - **ORPOLY #101**:
     - BSP player := (2640, 832, 448.1), yaw = 3π/2, i.e. facing BSP +X;
     - background → DUNGEON6;
     - RequestWorld(2).
   - **OSOUND #681** plays DOOR5CLS.
3. **Between frames:** projectiles destroyed, screen cleared, **mode 2**. From now on the orb is
   frozen.
4. **First BSP frames:** the player is 24.1 units above the floor surface, so airborne. Fall start
   ≈ 448.1.
   - Gravity starts on the frame after the airborne flag is first set.
   - The very first BSP frame since program start has dt = 0.
   - The player lands at z ≈ 424 after about 6–7 frames. The fall is < 100, so no hurt and
     no tag (no jump was pressed).
5. **Walk** about 9,800 units at 7.5 units per frame. That is about 1,300 frames, roughly 45 s at
   30 fps.
   - A hull-1 flood with step-up ≤ 18 and **no jumping** reaches the exit.
   - Coarse waypoints, origin z included:
     ```
     (2640,832,424) → (2864,832) → (2864,864) [down the stairs to ~104] → (2544,864,40)
     → (2544,512,−24) → (2512,432,−24) → (1776,432,−10) → (1776,−128,41) → (128,−128,88)
     → (128,−544,88) → (80,−576,104) → (80,−1312,88) → (−976,−1312,88)
     → (−2592,−1328,264) [long ramp up] → (−2592,−1104,309) → (−3056,−1104,424)
     → (−3056,−1312,424) → (−2528,−1376,424) → (−2528,−1136,424) → (−2416,−1136,424)
     ```
   - This route came from a 16-unit grid flood using the simplified rules above. It is a guide,
     not a recorded original path. **[M]**
6. **Enter the cube** |x+2384| ≤ 32, |y+1120| ≤ 32, |z−448| ≤ 32 (origin z 424 is inside). In the
   same frame:
   - **tag 134** (TagSwitchScan): OSETF COFFIN records "raise region COFFIN1's floor to 0 at
     speed 100" and its start sound. Motion waits for orb ticks.
   - **ORORB**: the orb player is teleported onto COFFIN1, i.e. (17272, 16784), z = −512,
     angle 1024 (facing −X), region 26, with COFFIN1's h/r/speeds where not −1.0;
   - background → TOCATTA;
   - RequestWorld(1).
7. **Between frames:** mode 1. **The orb resumes:**
   - the COFFIN1 floor rises from −512 to 0 and carries the player out of the pit;
   - crossing wall 127 would fire 134 again.
   - The level then continues normally.

Suggested end-to-end test for the reimplementation:
- Load LIPS00 and fire tag 132.
- Assert:
  - mode == BSP;
  - the BSP player is at (2640, 832, 448.1) and settles at z ≈ 424 within about 8 frames;
  - the heading is +X.
- Teleport the BSP player to (−2384, −1120, 424) and step one frame.
- Assert:
  - mode == orb;
  - the player is at (17272, 16784) in region 26 with angle 1024;
  - tag 134 fired once and the COFFIN1 floor starts rising toward 0.

---------------------------------------------------------------------------------------------------

## 7. Function map

| function | role |
|---|---|
| 0x43fc43 / 0x43fd81 / 0x44054b / 0x4403e9 / 0x440609 / 0x4406c7 | OLOADBSP / ORJUMP / ORORB / ORPOLY / OPTAGSW / ORTELE |
| FUN_0043fe61 | unreferenced: put the BSP player at info_player_start and RequestWorld(2); only called from the unbound key handler FUN_00477c59 |
| FUN_004b9981 | RequestWorld(1 or 2) |
| FUN_004db8a8 / FUN_004db8e8 | post a main-state flag / process state flags between frames (0x40 BSP load, 0x80 world switch) |
| FUN_0048fa06 / FUN_004ce9aa / FUN_00407b0b | LoadBsp / world+entities / BSP file parse (version 29, trigger models, bbox widen) |
| FUN_0048fc01 | unload BSP |
| FUN_0048f3fd | SetBspPlayer (position + heading from orb units) |
| FUN_0048f4ef / FUN_0048f597 | register orb-exit cube / tag-switch cube (max 120 each) |
| FUN_0048fdf1 / FUN_0049046a | exit-cube test / tag-switch test |
| FUN_00490295 | create type-3 marker entities |
| FUN_0048faf9 / FUN_004905d6 | BSP frame / simulate and draw |
| FUN_0048f679 / FUN_0049075c / FUN_00490bac / FUN_00490a4e | apply input bits / keyboard / mouse buttons / mouse look |
| FUN_004ceadf | entity tick (clock +5, dt) |
| FUN_00493d69 | player think |
| FUN_004cf559 / FUN_004cf27d / FUN_004cf13f / FUN_004cf477 / FUN_004cf300 | friction / gravity / integrate / accelerate-turn / jump |
| FUN_00410313 | ClipMove |
| FUN_0040fe0c / FUN_00410069 / FUN_0040fd16 / FUN_0040fb2c / FUN_0040fbd2 | UpDist / Trace / InEmpty / point-in-bbox / segment-bbox overlap |
| FUN_00468797 / FUN_004685a2 / FUN_0046861a / FUN_004688a2 | info_player_start / func_door scan / door entity / entity key lookup |
| FUN_004940b6 / FUN_0049c010 / FUN_00410d02 | attack / hitscan / trace vs models and entities |
| FUN_0049e701 | CheckRegionSound (background switch) |
| FUN_0042ce2d | MoveObjToObject (shared with OTELE) |
| FUN_004091dc / FUN_0048fcc0 / FUN_004912d7 | camera projection / BSP render init / palette loader |

## 8. Open points

1. The pitch sign on screen and how the projection's +0x50 term is used (vertical FOV). **[L/M]**
2. Where the "key held" table is cleared in mode 2. The orb-mode clear is skipped in mode 2, so
   held keys are presumably cleared on key-up messages; not traced. **[L]**
3. What the player controller's +0x248 object is (the hurt-sound holder in `Hurt()`). **[L]**
4. Whether other orb INI `[startup]` keys being re-read per level matters elsewhere. This document
   only relies on `loadbspfilename`.
