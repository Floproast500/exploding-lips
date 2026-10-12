# Tools, projectiles, explosions and the first-person weapon (lips.exe)

Clean-room notes on the G.O.D.D. "tool" system (the game's weapons) and the first-person weapon
overlay. Everything is paraphrased behaviour. Function addresses are the evidence.
Bot scripts are covered in `movers.md` and HUD text in `hud.md`. This file only covers the engine
side of bots shooting and being hit.

Units follow `physics.md`:
- World coordinates are 16.16. "u" = one world unit.
- Speeds are u per 1.2 s, so `step = v·dt_ms/1200`.
- Angles use 2048 per turn.
- **Countdown unit ("cu")**: many timers (object `+0x118`, anim-entry delays) lose
  `DAT_005670ec = 100·ticks/120` each frame, in 16.16. With 10 ms ticks, **1 cu = 12 ms**.

Confidence: **[H]** read from code, **[M]** inferred from code plus data, **[L]** guess.

Tool data is identical in all 18 level INIs (LIPS00–17). The section from `AVAILTOOLS` to
`[moveractioncodes]` diffs to zero. LIPSTART.INI has no tools.

---------------------------------------------------------------------------------------------------

## 1. Objects and tables

A tool definition is made of three **template objects**: the tool, its projectile and its
explosion. They are ordinary 480-byte objects. They are not placed in any region, and they are
cloned when used.

### 1.1 Creation from the INI (H)
- `OPTOOL[n]`, `OPPROJ[n]`, `OPXPLD[n]`, `OOTOOL[n]`, `OOPROJ[n]` and `OOXPLD[n]` are handled by
  FUN_0040ea54, FUN_0040ee69, FUN_0040ef72, FUN_0040f07b, FUN_0040f184 and FUN_0040f283.
- Each one calls the common parser FUN_0040e53a. The parser does the following:
  - Allocates a blank object (FUN_004d76a2: flags `0x01000000`, tag short `+0x46 = -1`).
  - Allocates an aux record, unless the text contains `*NOAUX*`.
  - Scans the text with the format at 0x51A26F (or the no-SNDS/NUMS variant at 0x51A30D).
- The handler then attaches the matching effect by name (FUN_00472847: "OPTOOL", "OPPROJ", …).
  This only looks the effect up and registers per-frame effects. It does **not** run the init
  handler of §1.4. The INI lines therefore carry the bits that init would have set themselves:
  - TY 0x100000 on tools, 0x200000 on projectiles, 0x400000 on OOTOOL;
  - AMF 0x80 on explosions;
  - beam bit 0x2000 and FL2 0x4000 on the laser.
  **[M]**
- Finally it stores the object in a per-index table:

| kind | table for team ≤ 0 (single player) | team 1 / team 2 tables | player ext copy for n = 0 |
|---|---|---|---|
| OPTOOL | `DAT_0053fbe4[n]` (mirror `DAT_00540964`) | `DAT_005409a4`, `DAT_0053d5a4` | ext+0x28 (and ext+0x20 = 0) |
| OPPROJ | `DAT_00543024[n]` (mirror `DAT_00542324`) | `DAT_005422e4`, `DAT_0053e2a4` | ext+0x2c |
| OPXPLD | `DAT_00542fa4[n]` (mirror `DAT_0053fce4`) | `DAT_0053fca4`, `DAT_0053d564` | ext+0x30 |
| OOTOOL | `DAT_0053fc64[n]` | … | `DAT_00543d04` |
| OOPROJ | `DAT_00543ca4[n]` | … | `DAT_00543d0c` |
| OOXPLD | `DAT_005422a4[n]` | … | `DAT_00543d08` |

- `n` is 0..15. The second "team == 1" branch in each handler is dead code, so team-2 tables are
  only filled from XRB objects.
- Each XRB also contains one set of placeholder template objects with index 0 (LIPS00 objects
  716–721, effects 90/91/92/95/96/97, texture names "OPTOOL"…). They are registered by the same
  init handlers. The INI definitions with the same index replace them. **[M]** (load order: the
  INI `[commands9999]` runs after the orb is loaded.)

### 1.2 Field → object offset (H, FUN_0040e53a)

| field | stored at | meaning (see §1.3 for which copies use it) |
|---|---|---|
| `[n]` | +0x54 | tool index |
| `TN` | +0x1ae (short) | team number (≤0 single player, 1/2 team tables) |
| `TEX` | +0x64 (texture/anim instance) | sprite texture loaded by name (FUN_00488fae) |
| `TY` | +0x00 | object flag word (§1.3) |
| `FL` | +0x04 | runtime state bits (normally 0) |
| `FL2` | +0x0c | more flags (§1.3) |
| `AMF` | +0x10 | flag byte; **0x80 = "counts as a weapon hit"** (§4.2) |
| `XF` | +0x1d0 | extra flags (copied to projectiles; 0 in all shipped data) |
| `R` | +0x3c | radius |
| `H` | +0x34 and +0x38 | height / full height |
| `MS` | +0x40 | **weight** (the field physics uses for pushing; ≥ 9000 = "heavy") |
| `MX(a b c d)` | +0x7c..+0x88 | max speeds msx/msy/msz/mst |
| `AC(a b c d)` | +0x8c..+0x98 | accelerations |
| `SNDS(s0..s4)` | aux +0x0e+12k → Sound refs aux +0x9c..+0xac | sound slots 0–4 (`~` = none) |
| `NUMS(n0..n4)` | aux +0x88..+0x98 | five ints, meaning depends on the kind (§1.5) |

### 1.3 Flag bits that matter (H unless noted)

**`TY` / object +0x00**
- 0x00000002: flying. No gravity. Vertical speed is damped toward 0 by g/2.
- 0x00000004: no friction.
- 0x00000010 / 0x00000020: collide with walls / with objects. Physics runs only if one of them
  is set.
- 0x00002000: **beam**. The projectile is launched at 9000 u/1.2 s (§3.2). Set on OPPROJ[6]
  (laser). The OPPROJ init also sets it when `NUMS[4] == 1`.
- 0x00010000 (byte +2 bit 0): **projectile marker**. The spawn code always ORs it in (see below).
  Bots treat any object with this bit as a weapon.
- 0x00020000 (byte +2 bit 1): **boomerang**. On impact the projectile returns to its owner
  instead of dying (OPPROJ[2] `01020032`; the OPBOOMA effect sets it).
- 0x00040000 (byte +2 bit 2): player-controlled (obj+2 & 4). Used by "is player" tests.
- 0x00100000 (byte +2 bit 4): set on tools by the OPTOOL init (FUN_0046608a). Means "when my
  animation reaches sequence code 99, set SHOOTPROJECTILE on my **owner**" (FUN_004a09fe).
- 0x00200000 (byte +2 bit 5): set on projectile templates by the OPPROJ init (FUN_00466379).
- 0x00400000 (byte +2 bit 6): OOTOOL marker (TY `01400030`; FUN_00466853 sets it). No reader
  found. **[L]**
- 0x01000000: always present (the "object exists" bit from the allocator).

Projectiles are **not** the template itself. FUN_00464e68 builds the new object's +0x00 as
`base | TY`:
- `base = 0x04010028` normally.
- `base = 0x0C01202E` if TY has bit 0x1 or 0x2000.
- 0x10 is added if TY has it.

So every projectile carries 0x00010000 (projectile marker) and 0x20 (collides with objects).
Beam projectiles also get 0x2 (flying) and 0x4 (no friction).

**`FL2` / +0x0c**
- Byte +0x0c bit 0x80: always ORed in on projectiles.
- 0x00004000 (byte +0x0d bit 0x40): **add to the LASER draw list** while flying (laser beam lines,
  §5.4). Set by the OPPROJ init when `NUMS[3] > 0`.
- 0x00008000 (byte +0x0d bit 0x80): on an explosion, "triggered" (internal).
- 0x00400000 (byte +0x0e bit 0x40): "tractor/grapple": the owner is attached to the explosion.
  Set by OPTBEAM. Unused in shipped data.
- 0x80000000 (byte +0x0f bit 0x80): **range limit** active (`NUMS[4] > 1`, §3.4).

**`AMF` / +0x10**
- 0x00000080: "weapon" marker for explosions. Set by the INI `AMF=00000080` lines; the XRB-object
  init forces it (FUN_004657a3 cases 5/6).
- 0x00400000 (byte +0x12 bit 6), on a **tool**: fire the object named in the tool's aux slot 9
  instead of the projectile, and spawn no explosion. Set by the OPTOOL init when NUMS[4]==1.
  Unused in shipped data. The spawned object then gets AMF 0x00800000.

### 1.4 Effect init handlers (H; they run for XRB-placed templates at level start, not for INI-created ones [M])

| effect | handler | does |
|---|---|---|
| OPTOOL | FUN_0046608a | Registers the tool. Sets TY bit 0x100000. `+0x118 = NUMS[0]`. NUMS[4]==1 → AMF bit 0x400000. An object named in aux slot 5 → tool-control flag +0x1d1 bit 8. Sets the AVAILTOOLS bit for its index (XRB objects only; the INI command does not). |
| OPPROJ | FUN_00466379 | TY bit 0x200000. NUMS[3]>0 → laser-list flag. NUMS[4]==1 → beam bit 0x2000. NUMS[4]>1 → range-limit flag. |
| OPBOOMA | FUN_00466722 | as OPPROJ, plus boomerang bit 0x20000; NUMS[0]>0 → +0x154 bit 1 |
| OPTBEAM | FUN_004667d5 | grapple flag (FL2 0x400000) |
| OPXPLD / OOXPLD | FUN_004669d2 / FUN_00466a35 | register; AMF \|= 0x80 |
| OOTOOL / OOPROJ / OOBOOMA / OOTBEAM | FUN_00466853 / 4668db / 466952 / 466992 | as the player versions, into the "other" tables |
| OTOOLCON | FUN_00466112 | tool resource check (§2.5) |
| OPROJCON | FUN_004664d0 | timed "fullbright" power (`DAT_0051081c`). Unused in shipped data. |

FUN_004657a3 (common registration):
- `NUMS[1] ≥ 1000` encodes a team number as `(NUMS[1]/1000)%1000`, then is reduced mod 1000.
- Non-tool kinds get `+0x124 = min(NUMS[1],10)` and `+0x11c = 0`.

### 1.5 What NUMS(n0..n4) mean, per kind (H unless noted)

| kind | n0 | n1 | n2 | n3 | n4 |
|---|---|---|---|---|---|
| tool (OPTOOL) | **ammo**: max uses, 0 = unlimited (tool+0x11c counts uses) | team code if ≥1000 | — | **1 = may only re-fire when the tool animation is idle** (0 = re-fire on cooldown alone) | 1 = "tool fires a named object" (AMF 0x400000) |
| projectile (OPPROJ/OOPROJ) | **bounces** left (wall/floor/ceiling ricochets before it explodes) | laser line **colour** (palette index, 0 → 199) | **tag fired when it hits the player** (bots set it from `P(n,tag)`) | laser line style: ones digit 1 = one centre line, 2 = two lines, 3 = both; tens digit = line spacing ×10 px (0 → 1) | 1 = beam (vx 9000); >1 = max range (≤10: ×radius, else in u) |
| explosion (OPXPLD/OOXPLD) | scale for owner speeds in grapple mode only | copied from the projectile on impact | tag for network-player hits only (decomp_extra ~27965) | copied from the projectile | 0 = destroy when the animation ends |
| tool-control (OTOOLCON) | `need = stateIdx + amount·100` | `delta = ±(stateIdx + |d|·100)` | fail tag | — | — |

Shipped values:
- Tools have `NUMS=(0 0 0 1 0)`: unlimited ammo and wait for idle. The laser (6) has
  `(0 0 0 0 0)`: no idle wait.
- The laser projectile has `(0 195 0 2 1)`: colour 195, two lines 10 px apart, beam.
- Explosions have `(0 0 3 0 0)`. The `3` only matters in network play.
- OOPROJ have `(0 0 3 0 0)`. Bots overwrite n2 with the tag on every shot (§6).

### 1.6 Which template fields reach the spawned objects (H)
- **Projectile** (FUN_00464e68) copies only these: TY (with base flags), FL2 (| 0x80), XF, TEX,
  MX[0..3] and the aux.
- These are **forced**: radius = 8 u, height = 16 u, FL = 0.
- AMF, MS (weight) and AC are not copied (they stay 0).
- So the INI `R`, `H`, `MS`, `AC` of OPPROJ lines have **no effect**. For example, OPPROJ[1]
  `R=16` is ignored.
- **Explosion** (FUN_00425910) is a full 480-byte copy of the OPXPLD template, then repositioned.
  All fields apply: R, H, AMF, MS, the texture and the aux.
- **Tool**: the template object itself is used, never cloned. It holds the first-person sprite
  animation and the per-tool use counter.

### 1.7 Shipped tool set (LIPS00 = all levels)

| # | tool TEX | projectile | explosion | notes |
|---|---|---|---|---|
| 0 | TOUCH (**no such tex in the XRBs**, nothing drawn) | BLACK, TY 01200030, MX0 60, MX2 60, SND0 SWOOSH02 | BLACK R16, AMF 80, NOAUX | default tool ("hand") |
| 1 | SWORD | BLACK, 01200030, MX0 60, MX2 200, SND0 SWORD05 | BLACK R32, AMF 80, SNDS HITFLESH ~ HITWALL | tool aux slot4 SWORD03 = "swing" sound |
| 2 | BOOMTHRW (TN -1) | BOOMRR 16 rot, 01020032 (flying + boomerang), MX0 500, SNDS SWISH01 ~ HITFLESH | none | returns to the thrower |
| 3 | BTHROW (TN -1) | BOMBPROJ, 01200030 (lobbed), MX0 500, SNDS SWISH01 ~ EXPLOD7 | MXPLD05 R48 H64, AMF 80 | splash bomb |
| 4 | CROSS99 (TN -1) | ARROW99 16 rot, 01200032 (flying), MX0 1200, SNDS ARRWSTRT ARRWFLY ARRWXPLD | BLACK R16, NOAUX | crossbow |
| 5 | SHOTGUN | BULLET, 01200032, MX0 9000, SND0 SHOT14 | BLACK R32, AMF 80, slot4 RICOCHT3 | one pellet only |
| 6 | LAZRGUN1 | BLACK, 01202036 (beam), FL2 4000, MX0 800, SNDS LASER1 ~ EXPLOD7, NUMS (0 195 0 2 1) | MXPLD04, TY 01000132, **no AMF** | auto-fire, laser lines |
| 7 | RIFLE99 | BULLET, 01200032, MX0 2000, SNDS ~ RIFLFLY RIFLEXPD | RIFLEXPD, no AMF | tool slot0 RIFLSTRT |
| 8 | MAGIC1 (TN 0) + `OTOOLCON[8]=(MAGIC,10,-5,674)` | FLAME02, 01200032, MX0 2000, SNDS ~ FLAME07 FLAME04 | MXPLD05, no AMF | costs 5 Magic, needs ≥10 |
| 9 | SNIPER0X | BULLET, 01200032, MX0 9999, SNDS SHOT14 EXPLOD8 RICOCHT3 | BLACK R16, AMF 80 | full-screen scope pic |

Bot ("other") set:
- `OOTOOL[1]` and `[2]`: TEX NO_NAME, TY 01400030.
- `OOPROJ[0..2]`: GREN01X, TY 01000030 (not flying → lobbed), MX0 1250, MX2 200, SNDS POP01 ~
  EXPLOD7.
- `OOXPLD[0]`: MXPLD04 R16, AMF 80.
- `OOXPLD[1]` and `[2]`: MXPLD01, AMF 80, NOAUX.
- There is **no OOTOOL[0]**, so a bot whose tool index is 0 cannot shoot (§6).

---------------------------------------------------------------------------------------------------

## 2. Player tool state, selection and activation

### 2.1 State (H)

| where | meaning |
|---|---|
| player ext (`obj+0x1b8`, static block 0x5b3028) **+0x20** | **current tool index** (0..15). The HUD "Tool" field prints this (decomp ~71665). |
| ext +0x28 / +0x2c / +0x30 | current tool / projectile / explosion template |
| player state (`obj+0x1b4`) +0x156 (short) | copy of the current tool index (also used as the *bot* tool index, §6) |
| `DAT_00543cfc` | AVAILTOOLS bitmask. Initialised to 1 (decomp ~66113). Every level INI sets `AVAILTOOLS=000003FF`, so **tools 0–9 are all available from the start of every level**. |
| player obj +0x118 | fire cooldown (16.16 cu) |
| player obj +0x20 bits 0/1/2 | ACTIVATETOOL / SHOOTPROJECTILE / TOOLDONESHOOT. Bits 1 and 2 survive the per-frame input reset (physics.md §3.1). |
| tool obj +0x11c | uses counter (ammo) |
| tool obj +0x144 | owner (set to the player on activation) |

- When the ext is first created (FUN_0048d123), the current tool = 0, so **the starting tool is
  tool 0 (TOUCH)**.
- On a level (re)load the static ext keeps its index. If that tool no longer exists, it falls back
  to 0 and bit 0 is forced on in AVAILTOOLS (decomp ~43529). **[M]**

### 2.2 Selecting (H)
- Keys **'0'..'9'**: the input sampler FUN_0047bfb0 (decomp ~37399) takes the last typed
  character. The bindable command `selecttool` (FUN_0047806a) does the same with its key code.
  Both use `index = char − '0'`.
- `FUN_0047b58f(player, n)` rejects the request if any of these hold:
  - n is −1;
  - n is already the current tool;
  - `DAT_0053fbe4[n]` is empty;
  - the AVAILTOOLS bit for n is clear.
- `FUN_0047b766(player, n, force)` does the switch:
  1. Ignore if the AVAILTOOLS bit is clear and `force == 0`.
  2. If changing, raise tag **920 + oldIndex** ("tool put away").
  3. Set ext+0x28/+0x2c/+0x30 from the tables. In team games use the team tables, with
     single-player fallback.
  4. Set ext+0x20 = n and state+0x156 = n.
  5. Raise tag **900 + n** ("tool selected").
- No next/previous-tool command or mouse wheel was found. The binding list near "activatetool"
  only has `activatetool` and `selecttool`.
- Firing: **Ctrl** and **left mouse button** are bound to `activatetool` by default
  (keyboard.txt, string "Ctrl = Use Tool"). It sets ACTIVATETOOL while held.
- Tag summary for scripts: 900+n selected, 910+n activated, 920+n deselected, 930+n "my projectile
  hit something". No shipped INI uses ONTAG 900–939.

### 2.3 Pickup effects (H)
All four are touch-triggered by the player (event 0x200 with a player-controlled bumper, or
script events 0x40/0x80). The tool index is the effect object's +0x54.
- **OSELTOOL** (FUN_00464ae9):
  - aux n1 == 0: select index (normal select).
  - aux n1 > 0: set the AVAIL bit, then force-select.
  - aux n1 < 0: clear the AVAIL bit, then force-select.
  - Then fire tag aux n0.
  - Shipped: LIPS00 obj 687 and LIPS17 obj 243 have no aux and index 5, so touching them selects
    the shotgun.
- **OTAVAIL** (FUN_00464c96):
  - aux n1 ≥ 0 (or no aux): set the AVAIL bit.
  - aux n1 < 0: clear the AVAIL bit.
  - Fire tag n0. Sound slot4 when n1 > 0, slot3 when n1 < 0.
- **OPAMMO** (FUN_0046493a): subtract aux n0 from the tool's use counter (clamped at 0), play
  slot4, fire tag n2.
- **OSETTOOL** (FUN_00464522):
  - Rebinds index n to the objects named in its aux slots: 8 → tool, 9 → projectile,
    7 → explosion.
  - If there is no aux, or no tool object, it clears index n.
  - Then selects n and fires tag n0 and 900+n.
  - Quirk: it writes state+0x156 = 0, not n.
- Not used by shipped data apart from OSELTOOL.

### 2.4 Activation and firing sequence (H — OPLAY per-frame handler FUN_0045bbe0, decomp_extra ~27770)

```
each frame (player):
  if cooldown > 0: cooldown -= DAT_005670ec           // obj+0x118
  tool = ext.tool
  if ACTIVATE and cooldown < 1 and tool and tool.aux and
     (tool.NUMS[3] < 1 or toolAnimIsIdle(tool)):       // FUN_004a08dc
       ActivateTool()                                  // FUN_00465da6
  if (SHOOT and cooldown < 1) or (SHOOT and DONESHOOT):
       if the player is grappled to an explosion: release first   // FUN_0045a6ee
       p = SpawnProjectile(player, yawOffset = view.lookLR, pitch = obj+0x28)  // FUN_00464e68
       clear SHOOT, DONESHOOT
       if p: if p.aux: play p.slot0 once, start p.slot1 loop   (launch / in-flight)
             else: play player aux slot3
       cooldown = 12.0 cu (144 ms)
       if tool.NUMS[0] > 0 and tool.uses >= tool.NUMS[0]: select tool 0 (out of ammo)
```

`toolAnimIsIdle` is true when the tool's current sequence entry points to itself (`next ==` own
index, e.g. entry 0 with next 0) or has next = 98 (hold).

`ActivateTool` (FUN_00465da6):
```
  if tool has no aux: cooldown = 30 cu; clear ACTIVATE; set DONESHOOT; return
  if tool.NUMS[0] > 0 and tool.uses >= tool.NUMS[0]: play tool slot0; return     // empty
  if tool has a tool-control object (OTOOLCON):
       if not ToolControl(controlObj): clear ACTIVATE; cooldown = 30 cu;
                                       fire control.failTag (674); return
  if not DONESHOOT:                       // normal case
       tool.owner = player
       start tool animation action 1      // FUN_004a0c61(tool+0x64, 1)
       play tool aux slot0                // e.g. RIFLSTRT
  else:
       set SHOOT                          // fire immediately (see note)
  if tool.NUMS[0] > 0: tool.uses += 1
  raise tag 910 + ext.toolIndex
  cooldown = 30 cu (360 ms); clear ACTIVATE; set DONESHOOT
  play tool aux slot4 if any (SWORD03 on the sword), else player aux slot3
```

**Release timing.**
- The projectile is released when the **tool animation reaches a sequence entry whose `next` is
  99**. The anim stepper FUN_004a09fe sees code 99, sets SHOOTPROJECTILE on the tool's owner
  (the tool has TY bit 0x100000) and continues with the following entry.
- On the next OPLAY tick, SHOOT and DONESHOOT are both set, so the projectile spawns.
- The anim is stepped once per **rendered** frame by the first-person view code (§5).

Consequences of the DONESHOOT toggle (derived from the code paths, **[M]**):
- Tools whose animation has a 99 entry fire once per press, at the 99 frame.
- Tools with no 99 entry (TOUCH has no texture at all; SNIPER0X is a single static pic):
  - The first activation only "arms" DONESHOOT.
  - The next activation fires immediately.
  - So they fire on every **second** press. Holding the key fires about every 720 ms.
- The laser (NUMS[3] = 0, no idle wait), with the key held:
  - The activation every 360 ms alternates between "restart the fire animation" and "fire now".
  - So it auto-fires about every 504 ms (360 + 144 ms) and never reaches its own 99 frame.
  - A single tap fires at the 99 frame, 900 ms after the press.

### 2.5 OTOOLCON resource check (H — parser FUN_0040eb6f, check FUN_00466112)
- `OTOOLCON[n]=(STATE, need, delta, failTag)`. It must follow `OPTOOL[n]`.
- The parser maps STATE to an object-state index 1..5 with FUN_0044b345: Life 1, Magic 2,
  Armor 3, Money 4, Experience 5. A bad name prints "OTOOLCON invalid state name".
- It stores the encoded values in a fresh control object's aux, sets the tool's +0x1d1 bit 8 and
  +0x128 = control object.
- Check, run on the player's object state (`state+0x198 + 4·idx`):
  - If `value < need`: play control slot1 and fail.
  - Otherwise, if delta ≠ 0: `value += delta` **directly** (no threshold tags), play control
    slot0, and succeed.
- For tool 8: needs Magic ≥ 10, costs 5. On failure tag 674 fires (ONTAG[674] → "You need more
  magic for this tool"), and the press is ignored for 30 cu.
- The cost is paid on activation, before the animation plays.

---------------------------------------------------------------------------------------------------

## 3. Projectile life cycle

### 3.1 Spawn (H — FUN_00464e68(shooter, angleOffset, pitch))

1. **Position**:
   ```
   a  = shooter.yaw + angleOffset           // player: view look-left/right offset; bots: 0
   d  = shooter.radius + 4 u                // player r=16 → 20 u in front
   xy = shooter.xy + (cos a, sin a)·d
   z  = player: clamp(eyeZ − 16 u, feetZ, feetZ + Height)   // 32 u above the floor standing,
                                                            // the floor when crouched
        bot:    obj+0x30 + Height/2         // +0x30 = feet z for non-players → mid-body
   ```
   If (xy, z) is in no region, nothing is fired.
2. **Templates**:
   - Player: ext tool/proj/xpld (only if the AVAILTOOLS bit of the current index is set).
   - Bot: OOTOOL/OOPROJ/OOXPLD at index `state+0x156` (or `DAT_00543d00` = 0 without a state).
   - No tool or no projectile template → nothing.
3. Clone the projectile as in §1.6:
   - owner +0x144 = shooter; team +0x1ae = shooter's team.
   - yaw = a.
   - effect = OPROJ (handler FUN_0045f421).
   - name "P_PROJTL".
   - `+0x128 = NUMS[0]` (bounces).
4. **Velocity** (vx = forward +0xac, vz = +0xb4):
   ```
   vx = MX[0] + shooter.vx                  // the player's walk speed (800) is ADDED when moving
   if shooter is a bot:
        vz = MX[2]/2
        if |player.z − shooter.z| ≤ 16 u: raise the spawn z by that difference (aim at player height)
   elif not (TY & 2) and has aux:   vz = MX[2]/2      // lobbed (bomb, touch, sword)
   else:                            vz = vx · pitch / 280   // aimed along view pitch (±512)
   if TY & 0x2000 (beam):  vx = 9000;  vz = 7·vz;  clear 0x2000 from the copy
   ```
   The initial speeds are saved at +0xbc..+0xc8. The template MX values are saved at
   +0xcc..+0xd8.
5. **Explosion**:
   - If an explosion template exists (and the tool lacks AMF 0x400000), a hidden explosion object
     is pre-created now.
   - It is a full copy of the template at the spawn point, with effect OXPLD, +5 |= 0x30 (hidden)
     and owner = shooter.
   - It is stored in projectile +0x18c.
6. The flag `DAT_00510770` is cleared. It would show the projectile's animation as a
   first-person overlay, but nothing ever sets it (only three references in the binary). **Dead
   code.**

### 3.2 Flight (H for the dispatch, M for the generic physics details)
- Projectiles move with the generic non-player physics FUN_004a102a. There is no thrust (their
  action word is 0).
- **Gravity**: if TY bit 2 is clear (`…30` types: bomb, touch, sword, bot grenades), vz loses g
  (level g = 400) per 1.2 s. These arc.
- **Flying** types (`…32`: boomerang, arrow, bullets, flame, laser): vertical speed is damped
  toward 0 by g/2. They fly straight.
- **Friction**: projectiles without the no-friction bit lose fx (200, possibly halved while
  airborne) from vx per 1.2 s. Only the laser has no-friction. In practice they hit something
  first.
- Collisions use radius 8 u and height 16 u with the usual sub-stepping (≤ 8 u per step), so fast
  bullets do not tunnel.
- Each frame the OPROJ handler (event 0x10) does the following:
  - If vx, vy and vz are all 0, or there is no region, or the range limit is exceeded (§3.4):
    "stop" (§3.3).
  - Else if FL2 0x4000 is set: (re)add itself to the LASER list (§5.4).
  - In state 2 ("impacted"): one frame later, undo the region light flash. Then become a static
    prop if the impact animation holds (code 98; no shipped projectile does), otherwise destroy
    itself.
  - Keep a carried object (boomerang) glued to the projectile.

Approximate ranges (vx in u/1.2 s; no shooter speed added):
- Touch: about 49 u in front of the player centre (falls 32 u while moving at 60).
- Sword: about 63 u.
- Bomb: about 330 u (lob: peak +12.5 u, lands after about 0.85 s).
- Arrows and bullets: until they hit a wall.

Moving forward adds 800 to vx, so a sword swing while walking travels much farther. **[M]**

### 3.3 Hits (H — OPROJ events, FUN_0045ebe8 "impact", FUN_0045e91e "boomerang return")

| event | condition | result |
|---|---|---|
| 0x100 wall | bounces left (`+0x128 > 0`) | bounces−−. Yaw is mirrored about the wall (FUN_0045e80a). Play own slot4 unless the wall has byte1 bit 0x20. |
| 0x100 wall | no bounces | Play explosion slot2 (normal wall) or slot3 (wall byte1 & 0x20). Then **Impact**. |
| 0x400 floor / 0x800 ceiling | bounces left | Not flying: `vz = ±MX[2]·bouncesLeft/NUMS[0]` (decaying bounce); flying: vz = −vz. Bounces−−, play slot4. |
| 0x400 / 0x800 | no bounces | **Impact** |
| 0x200 object X | X is the owner | ignored (you cannot shoot yourself) |
| 0x200 object X | X is non-solid (byte1 & 2) and not +0x1d3 & 1 | ignored |
| 0x200 object X | otherwise | **Impact**. Then play explosion slot0 if X.weight < 9000 ("flesh"), else explosion slot1. |
| stop (speed 0 / range) | not boomerang | Destroy. Stop slot1, play own slot3. Trigger the explosion **at its pre-created spawn position** (no copy of the projectile position on this path, **[M]**, probably an original bug). |
| stop | boomerang | start the return |

**Impact** (FUN_0045ebe8):
```
  stop own slot1 (flight loop); play own slot2 (impact sound) at X
  if owner has a player ext: raise tag 930 + owner's current tool index
  if returning boomerang (state 1): drop the carried object, destroy self
  elif state 0:
     if not boomerang:
        region light −= 2 (floor, ceiling, all walls)  → one-frame brightening "flash"
        vx = vz = 0; state = 2; start own tex action 1; set translucent (byte1 bit0)
        clear TY 0x2000/0x30 (no more movement/collision)
        explosion: move it to the projectile position (FUN_0045ea73, also copies NUMS[1],[3]),
                   un-hide, mark triggered, link into the projectile's region
     else: start the boomerang return
  if NUMS[2] > 0 and X is player-controlled (obj+2 & 4): raise tag NUMS[2]     // bot damage tag
```

**Boomerang** (TY 0x20000):
- On its first impact or stop it enters state 1. It turns 180° and steers home with the path
  follower FUN_0045a943(owner, mode 3).
- If it hit a light object (weight < 9000 or flying) and `NUMS[3] == 0`, it **carries** that
  object back: the object's byte1 |= 4 and it is glued to the projectile.
- On touching its owner it is destroyed. The carried object receives a 0x200 "bumped" event, so
  pickups are collected.
- Sounds: slot2 when it turns, slot1 keeps looping.
- A returning boomerang still has the projectile marker, so it can hit bots again on the way
  back. **[M]**

### 3.4 Range limit (H, FUN_0045f339)
- Only with FL2 0x80000000 (OPPROJ NUMS[4] > 1).
- The projectile stops when its horizontal distance from the spawn point (+0x18) is at least
  `NUMS[4]·radius` (if NUMS[4] ≤ 10) or `NUMS[4]` u (otherwise).
- Not used by shipped data.

---------------------------------------------------------------------------------------------------

## 4. Explosions and damage

### 4.1 Explosion object (H — OXPLD handler FUN_0045ef76)
- On trigger:
  - Show it.
  - Start texture action 0.
  - Flags |= 0x08000200.
  - Set +0x118 = 1 (exploding).
- Each frame while exploding, the animation is checked with FUN_004a0962. It is "finished" when
  the current entry points to itself or has next = 0.
- When finished:
  - next == 98: become a static decoration (flags = 1, no effect).
  - Otherwise, if the aux NUMS[4] == 0: fire the object's own tag (+0x46, −1 by default → none)
    and destroy it.
- Lifetimes of the shipped explosions (sum of the delays until the self-looping entry):

| texture | used by | lifetime |
|---|---|---|
| BLACK | OPXPLD 0,1,4,5,9 | 1 frame (entry 0 has next 0) |
| MXPLD05 | bomb (3), magic (8) | 205 cu ≈ 2.5 s |
| MXPLD04 | laser (6), OOXPLD[0] | 150 cu ≈ 1.8 s |
| RIFLEXPD | rifle (7) | 90 cu ≈ 1.1 s |
| MXPLD01 | OOXPLD 1,2 | 150 cu ≈ 1.8 s |

### 4.2 How bots get hurt (H for the flag test, M for the counting)
- A bot only reacts to object bumps (event 0x200; OTHING → FUN_00448e8b / FUN_00448d8c).
- The bumper counts as a **player weapon** when **both** hold:
  - it has the projectile marker (obj+2 bit 0 = TY 0x10000) **or** the AMF weapon bit
    (obj+0x10 & 0x80);
  - its owner (+0x144) is the player.
- Then the mover state gets HP (byte +4 bits 0x08|0x40).
- Projectiles always have the marker, because spawn ORs in 0x04010028.
- TY values in the INI (`01200030` …) do **not** contain 0x10000 themselves. The marker comes
  only from the spawn base.
- Explosions only qualify through AMF 0x80. In the shipped INI that is OPXPLD 0, 1, 3, 4, 5, 9 and
  every OOXPLD.
- OPXPLD 6, 7 and 8 (laser, rifle, magic; `AMF=0`) are **harmless** visuals, because the INI path
  does not run the init that would force the bit.
- Explosion owner = shooter, so player explosions count and bot explosions never hurt bots.
- **Damage model**: there are no damage numbers in the engine. Each HP event decrements the bot's
  hits-to-kill counter (`state+0x1b0`, set by SH(n)) by 1 (FUN_00448fcc). See movers.md.

Contacts per player shot (**[M]**):
1. The projectile itself, once. On the first contact it impacts and drops its collision bits.
2. The explosion appears at the same spot in the same frame. Its radius is R (16/32/48).
   - It collides like any object with the 0x30 bits for as long as it lives, so every
     overlapping bot can get repeated 0x200 bumps.
   - The bot's own 20 cu (240 ms) debounce at obj+0x118 (movers.md) filters them.
   - So the 1-frame BLACK explosions add nothing after the projectile hit.
   - The bomb's MXPLD05 (R 48, H 64, 2.5 s) can register about 1 + 2.46/0.24 ≈ 10 hits on a bot
     standing in it.
   - The laser, rifle and magic explosions do not count.
   - Whether a stationary object generates bumps every frame was not traced in FUN_004a4a1f
     **[L]**. A bot moving into the explosion certainly does.
3. Melee tools 0 and 1 are ordinary short-range lobbed projectiles (§3.2). They follow the same
   rules.

### 4.3 How the player gets hurt (H)
- Single player has **no hit-point code** for weapons. The only player damage is the **tag** in
  the projectile's aux NUMS[2], fired by Impact when it hits a player-controlled object (§3.3).
- Bots set this tag per shot from `P(n,tag)`. ONTAG[411..419] apply
  `SUMOS(-5…-90, Armor, Life)` and a growl/message.
- Team/network scoring (FUN_00454956) is separate and only active in team games.
- `ON BP { XT(661) }` (touching a bot) is pure script: tag 661 → ONTAG[661]. It is not an engine
  weapon.

---------------------------------------------------------------------------------------------------

## 5. First-person weapon display

### 5.1 Where (H)
- The main view's post-render effect FUN_00456e36 (event 0x20, called after the 3D view) does the
  following every rendered frame:
  1. Draws the laser lines (§5.4).
  2. Steps the current tool's sprite animation: `FUN_004a09fe(tool, tool+0x64)`.
  3. Draws the current pic with `FUN_0041afd1(tool+0x64, cx = screenW/2, cy = screenH + 8,
     light 0)`.
- `screenW/H` = `DAT_005486a4/ac` = 320×200 (640×480 in the hi-res mode).
- The same draw also appears in FUN_004905d6, another render path.
- **No bobbing.** A cosine of `DAT_00510748` is computed just before the draw and discarded, and
  `DAT_00510748` is zeroed at the top of the handler. Nothing depends on walk phase or speed.
- **No sway, no scaling with distance.**

### 5.2 Placement and scaling (H, FUN_0041afd1)
- Anchor is the pic hotspot: `left = (cx − hotX)/sx`, `top = (cy − hotY)/sy`, size `w/sx × h/sy`.
- `sx, sy` = view +0x84/+0x88 (buffer/view scale; 1.0 for the full 320×200 view). So the weapon is
  authored in 320×200 screen coordinates and shrinks with the view window.
- Clipped to the view rectangle, and to per-column limits (view+0x6c) when present.
- Texel 0 is transparent.
- Light: shade row 0 (full bright) of the view's shade table. When the fullbright power flag
  `DAT_0051081c` is on, light `DAT_0054868c` is used instead.
- All tool pics have `hotY = h − 1` and `hotX = w/2`, so the **pic's bottom row lands at
  y = 208**. The bottom 8 rows are always off-screen and the pic is horizontally centred.
  - 320×200 pics: rows 0..8 hidden at the bottom; visible top at y = 9.
  - SWORD (320×240): top at −31.
  - SNIPER0X (320×220): top at −11, covering the whole view.

### 5.3 Animations (data from LIPS00.XRB Texs; all rotations = 1)
- Action 0 = idle. It starts at entry `starts[0]` = 0, always "entry 0 → next 0" except the laser.
- Action 1 = use. It starts at `starts[1]`.
- Delays are in cu (×12 ms).
- "Release" = the entry whose next is 99. The projectile spawns on the next game tick.

| tool | idle pic | use sequence (entry:frame/delay) | release after | back to idle after |
|---|---|---|---|---|
| SWORD | f0 (320×240) | e1..e5 f1–f5 @5 → 99, e6 f3, e7 f1 @5 | 25 cu = 300 ms | +10 cu |
| BOOMTHRW | f0 = blank pic 816 (2×256) | e1..e5 f1–f5 @5 (growing 40×24…112×108) → 99, e6 f3, e7 f1, e8 blank @80 | 300 ms | +90 cu (no re-throw for about 1.1 s) |
| BTHROW | blank | e1 f1 @10 → 99, e2..e7 @10/15, e8 blank @100 | 120 ms | +165 cu |
| CROSS99 | f0 | e1 @0, e2..e4 f1–f3 @5 → 99, e5 f4 @40, e6 f5 @5, e7 f0 @40 | 180 ms | +85 cu |
| SHOTGUN | f0 | e1 f0 @10, e2 f1 @10 → 99, e3..e6 @10, e7 f0 @60 | 240 ms | +100 cu |
| LAZRGUN1 (54×48 pics) | **animated idle** e0..e5 f0,1,2,3,2,1 @50 (loop) | starts at e6: e6..e10 f4–f8 @15 → 99, e11..e13 @15 → e0 | 900 ms | +45 cu |
| RIFLE99 | f0 | e1 f0 @5, e2 f1 @5 → 99, e3..e5 @5 → e0 | 120 ms | +15 cu |
| MAGIC1 | blank (frame 19 = pic 816) | e1..e15 f0–f14 @10 → 99, e16..e19 @10 | 1.8 s | +40 cu |
| SNIPER0X | single static 320×220 pic, no use animation, no 99 | — | see §2.4 | — |
| TOUCH | texture missing → nothing drawn, no 99 | — | see §2.4 | — |

Notes:
- The anim stepper subtracts `DAT_005670ec` once per rendered frame. When the countdown drops
  below 1 it follows `next`: 98 = stay, 99 = signal and go to entry+1. Then it adds that entry's
  delay.
- Each tool object keeps its own animation state, so a tool switched away mid-swing resumes where
  it was.
- The blank pic 816 is a 2×256 transparent strip. "Blank" idle means no hand is visible.

### 5.4 Laser lines (H, inside FUN_00456e36)
For each object in the LASER list (`DAT_0056e96c`; projectiles with FL2 0x4000 while flying):
- colour = aux NUMS[1], or 199 if 0;
- style s = NUMS[3] % 10 (0 → 1);
- spacing k = (NUMS[3]/10) % 10 (0 → 1).

What is drawn:
- **Player-owned**:
  - s = 1 or 3: a line from (160, 199) to the screen centre (screenW/2, screenH/2).
  - s = 2 or 3: two more lines from (160 − 10k, 199) and (160 + 10k, 199) to the centre.
  - The x of 160 and the 199 are hard-coded, as in the 320×200 layout.
- **Bot-owned**: the same pattern, from the owner's projected screen point (+0x1ca/+0x1cc, ±8 px)
  to the projectile's projected point.
- The shipped laser (`NUMS=(0 195 0 2 1)`) draws two colour-195 lines from (150,199) and
  (170,199) to the screen centre. They show for as long as the 9000-speed projectile flies,
  usually one or two frames.

---------------------------------------------------------------------------------------------------

## 6. Bots shooting (engine side; scripts in movers.md)

- **`P(n[,tag])`**, the mover interpreter action at decomp ~20430 (inside FUN_004470f2):
  1. Store `n` into mover state +0x156 (the bot's tool index). Store tag (default −1).
  2. Try to start body animation action **7** (`FUN_004a0d29(obj, anim, 7)`). Every shipped bot
     texture has `starts[7] == 0`, so the call reports "no attack anim".
  3. **Fire immediately**: `SpawnProjectile(bot, 0, 0)`.
  4. If the projectile has an aux, set its **NUMS[2] (+0x90) = tag**.
  5. If the bot is in the player's region, play the projectile's slot0 (POP01) once and slot1.
- If a bot had an action-7 animation, its byte +0x12 bit 0 would mark "shot pending". The shot
  would then fire when the animation reaches a 99 entry: the bot's own SHOOT bit, then OTHING
  FUN_00425361 calls `SpawnProjectile` without setting the tag, so the template NUMS[2] (3) would
  be used. Not exercised by shipped data. **[M]**
- `n` selects OOTOOL[n]/OOPROJ[n]/OOXPLD[n]:
  - `P(1,41x)` → index 1: a grenade (GREN01X) lobbed straight along the bot's facing.
    - vx 1250 (plus the bot's own forward speed), vz 100 up, gravity 400.
    - Spawn at mid-body height, lifted by up to 16 u toward the player's height.
    - There is **no aim at the player** beyond the bot's yaw. The mover's face/track actions turn
      it first (e.g. `F(16)` before `P`).
  - `P(2[,tag])` → index 2: the same grenade. Without a tag the projectile's NUMS[2] is −1, so it
    **does no damage** when it hits the player. `DETHBOT9`'s `P(2,419)` does carry a tag.
  - `P(0,…)` would not fire, because OOTOOL[0] is undefined.
- **On hitting the player**: Impact → tag NUMS[2] (411…419) → ONTAG → `SUMOS(-x, Armor, Life)`.
  - One tag per projectile.
  - Hitting a wall or the floor fires no tag.
  - The OOXPLD explosion (owner = bot) never hurts the player in single player.
- **Rate of fire** is set only by the mover script timing (`W(16)`/`MD(16)` between shots).
  Bots have no engine cooldown on `P`.

---------------------------------------------------------------------------------------------------

## 7. Reimplementation summary

```rust
// ---- data -------------------------------------------------------------------------------
struct ToolDef   { tex: Tex, nums: [i32;5], snds: [Snd;5], control: Option<ToolCon> } // OPTOOL
struct ProjDef   { ty: u32, fl2: u32, tex: Tex, mx: [Fx;4], nums: [i32;5], snds: [Snd;5] } // OPPROJ
struct XpldDef   { obj_template: Object /* full copy: R,H,AMF,tex,aux */ }               // OPXPLD
struct ToolCon   { state_idx: usize, need: i32, delta: i32, fail_tag: i32 }              // OTOOLCON

// ---- per-frame player logic (run inside the 33 ms game tick) ------------------------------
fn player_tools(p: &mut Player, dt_cu: Fx /* 100*ticks/120 */) {
    if p.cooldown > 0 { p.cooldown -= dt_cu; }
    let t = &mut tools[p.cur];
    if p.act && p.cooldown < 1.0 && t.has_aux && (t.nums[3] < 1 || t.anim.is_idle()) {
        activate(p, t);
    }
    if p.shoot && (p.cooldown < 1.0 || p.done_shoot) {
        if let Some(pr) = spawn_projectile(p.obj, view.look_lr, p.pitch) {
            sound(pr.snds[0]); sound_loop(pr.snds[1]);
        }
        p.shoot = false; p.done_shoot = false;
        p.cooldown = 12.0;
        if t.nums[0] > 0 && t.uses >= t.nums[0] { select_tool(p, 0, false); }
    }
}
fn activate(p, t) {
    if t.nums[0] > 0 && t.uses >= t.nums[0] { sound(t.snds[0]); return; }
    if let Some(c) = t.control {
        let v = &mut p.state[c.state_idx];
        if *v < c.need { sound(c.snds[1]); p.act=false; p.cooldown=30.0; fire_tag(c.fail_tag); return; }
        *v += c.delta;                               // no threshold processing
    }
    if !p.done_shoot { t.owner = p; t.anim.start(1); sound(t.snds[0]); } else { p.shoot = true; }
    if t.nums[0] > 0 { t.uses += 1; }
    fire_tag(910 + p.cur);
    p.cooldown = 30.0; p.act = false; p.done_shoot = true;
    sound(t.snds[4].or(p.snds[3]));
}
// tool anim stepped once per rendered frame; entry.next==99 → owner.shoot = true, go entry+1

// ---- projectile ---------------------------------------------------------------------------
fn spawn_projectile(sh, ang_off, pitch) -> Option<Proj> {
    let (tool, pd, xd) = if sh.is_player { cur_templates() } else { other_templates(sh.state.tool) };
    let a = sh.yaw + ang_off;
    let pos = sh.xy + dir(a) * (sh.radius + 4.0);
    let z = if sh.is_player { clamp(sh.eye_z - 16.0, sh.z, sh.z + sh.height) }
            else            { sh.z + sh.height / 2.0 };
    region_at(pos, z)?;
    let mut o = Object::blank();
    o.flags = (if pd.ty & 0x2001 != 0 { 0x0C01202E } else { 0x04010028 }) | (pd.ty & 0x10) | pd.ty;
    o.radius = 8.0; o.height = 16.0; o.owner = sh; o.team = sh.team; o.yaw = a;
    o.max = pd.mx; o.bounces = pd.nums[0];
    o.vx = pd.mx[0] + sh.vx;
    o.vz = if !sh.is_player { pd.mx[2] / 2.0 }          // + lift z toward player if |dz| ≤ 16
           else if pd.ty & 2 == 0 && pd.has_aux { pd.mx[2] / 2.0 }
           else { o.vx * pitch as f32 / 280.0 };
    if o.flags & 0x2000 != 0 { o.flags &= !0x2000; o.vx = 9000.0; o.vz *= 7.0; }
    o.xpld = xd.map(|x| x.spawn_hidden(pos, z, a, owner = sh));
    Some(o)
}
// per frame: generic physics (gravity unless flying, friction unless 0x4), r=8, h=16
// on wall/floor/ceiling: bounce while bounces>0 (sound slot4) else impact()
// on bump(X): if X==owner ignore; impact(); sound(xpld.snds[if X.weight<9000 {0} else {1}])
// on speed==0: destroy (slot3), trigger explosion at its spawn point (original quirk)
fn impact(o, x) {
    stop(o.snds[1]); sound_at(x, o.snds[2]);
    if o.owner.is_player { fire_tag(930 + cur_tool); }
    if o.boomerang { if o.returning { o.destroy() } else { o.start_return(x) } }
    else {
        o.region.flash(-2 /* one frame */); o.stop(); o.anim.start(1); o.no_collide();
        if let Some(e) = o.xpld { e.pos = o.pos; e.show(); e.anim.start(0); }
        o.destroy_next_frame();
    }
    if o.nums[2] > 0 && x.is_player { fire_tag(o.nums[2]); }   // bot attack damage
}
// explosion: lives until its action-0 sequence reaches a self-looping entry, then destroyed.
// It counts as a player weapon (AMF 0x80) for bots it touches; the bot's 240 ms debounce applies.

// ---- weapon overlay (after 3D view, before HUD) -------------------------------------------
fn draw_weapon(view) {
    draw_laser_lines(view);
    let t = &mut tools[player.cur]; t.anim.step(dt_cu);
    if let Some(pic) = t.anim.pic() {
        let (sx, sy) = view.scale;                        // 1.0 at 320x200
        let left = (160 - pic.hot_x) / sx; let top = (208 - pic.hot_y) / sy;
        blit_transparent(view, pic, left, top, pic.w / sx, pic.h / sy, shade_row = 0);
    }
}
```

HUD: the "Tool" value is the current tool index `ext+0x20`. The state copy at +0x156 is the same,
except after OSETTOOL. Show 0..9.

---------------------------------------------------------------------------------------------------

## 8. Open questions

1. **Explosion collisions while stationary.** Do explosion objects run the object-collision
   pass every frame with zero displacement (FUN_004a4a1f)? This decides how many splash hits a
   standing bot takes from the bomb. Measure in Wine by counting kills with the bomb.
2. **Init for INI templates.** FUN_00472847 does not call the init handlers. Confirm that nothing
   else sends them an init event later (e.g. a global effect-init pass at level start that also
   walks INI-created objects). If something does, OPXPLD 6/7/8 would become splash weapons
   (case 5 forces AMF 0x80).
3. Explosion **sound slot 4** (MXPLD4, MXPLD7, FLAME04, RICOCHT3) and the tool's slot 4 for
   non-sword tools: no player found for the explosion's slot 4 yet. It is probably played by the
   generic object/animation sound code when the explosion appears.
4. Exact generic-physics friction for projectiles (halved while airborne?) and the
   vertical damping of flying projectiles. These are only needed for long-range accuracy.
5. Whether `INITTOOL`/`TOOLSOUT` (console commands: FUN_0040e429 resets tool tables, prints
   AVAILTOOLS) matter for level transitions.
6. The "stop" path triggers the explosion at the projectile's spawn point. Confirm in game: fire a
   bomb onto a flat floor and see whether it explodes at the floor (impact path) or back at the
   player.
7. The DONESHOOT alternation for TOUCH and SNIPER (fires on every second press) is derived from
   code. Confirm in game.
8. Persistence of the selected tool across level changes: the static ext block is kept, but a
   save/load restore at decomp ~44052/67114 may rewrite it.
