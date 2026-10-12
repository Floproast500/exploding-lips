# Wall and region effects, texture animation (lips.exe)

Clean-room notes for the G.O.D.D. engine (`lips.exe`, Watcom, Dec 1999). They cover the wall and
region effect handlers used by the shipped levels, and how world textures animate. Everything is
paraphrased as prose and pseudocode. The addresses are evidence only.

Companion docs: `effects.md` (registration table), `../formats/XRB-records.md` (record layouts),
`script.md` (tags), `physics.md` (time scale, collision, jump pads), `renderer.md`.
Data cross-check: `python3 -I re/scripts/wallregion_dump.py [original] [--list NAME | --anim]`.

Confidence: **H** = read directly from code and confirmed by data; **M** = read from code but some
paths not fully traced, or inferred from code plus data; **L** = guess.

Units: heights and speeds are 16.16. "u" means one world unit. Angles use 2048 units per turn.
`Scale(v)` is the engine's per-frame time scaling (`FUN_00473195`, physics.md §2):
`Scale(v) = v * ticks / 120`, where `ticks` is the number of 10 ms engine ticks this frame (1..20).
For |v| < 1024 u the fraction of `v` is dropped first (table lookup). So a "speed" of S means
S u per 1.2 s, which is S/1.2 u/s. At the 30 fps cap (3 ticks) it moves S/40 u per frame.

---

## 0. Most important finding: the XRB is a post-init snapshot (H)

Each effect has an init branch (event 1). It converts editor values into runtime state: it shifts
speeds `<<16`, closes doors, records rest heights in the vars, sets flag bits and saves the original
geometry. **The compiled `.XRB` files were written after that init had run.** Evidence from all
19 levels:

- All 156 RTDOOR regions are stored closed (ceil == floor). Var C equals the rest ceiling (+0x18),
  the closed flag (region+4 bit 0x800000) is set, and +0xa0 already holds a 16.16 speed (200.0,
  50.0, ...).
- All RFUP1 regions have C == floor and D == −1 (the init values).
- All RFBOUNCE regions have region flag 0x200 set and A == aux+0x88 << 16.
- All RROTATE regions have +4 == 0x90, B == aux+0x88 and +0x7c == aux+0x8c. Their pivot point is
  stored at +0xf8. All 102 walls with a non-null wall+0x14 belong to RROTATE regions, and that point
  equals p1: it is the saved original geometry.
- All WTAGSW walls carry flag 0x2000 (the init sets wall byte 1 bit 0x20). All WTEXHOR walls have
  +0xbc = speed << 16.

Event 1 is broadcast only by the source-zone loader `LoadZone` (`FUN_0047cb09` → `FUN_00472f90(1)`).
The compiled loader `FUN_004d5968` does not broadcast it. The post-load pass `FUN_0047f7a5` calls
only the RSLANT handler with event 1, for regions that already have the "slanted" bit
(region byte 2 bit 0x10). That call re-resolves one pointer (see RSLANT).

**Consequence for the reimplementation:** do not run the init branches when loading an XRB. Use the
stored values as runtime state. The init pseudocode below is given only to explain the stored values.

---

## 1. Effect dispatch (H)

**Handler call.** `handler(entity, event, activator)`. Watcom registers: EAX = wall/region/object,
EDX = event code, EBX = activator object or 0.

**Masked dispatch** (`FUN_00472a54(effect, entity, event, activator)`). The handler is called only if
all of these hold:
- `effect.flags & event` is non-zero (the flags column in effects.md);
- the entity is not disabled. Disabled means `(byte3 & 1 && byte4 & 1)` or `byte7 & 0x80`. For walls
  and regions, byte3/byte4/byte7 are bytes of the u32 flags at +0 and +4.

**Active list.** The fix-up `FUN_004d21fa` calls `FUN_0047291d` for every object, wall and region.
That call appends `(effect, entity)` to the active list when the effect flags contain 0x10 or 0x08.

### Event codes

| event | meaning | sender |
|---|---|---|
| 0x01 | init (editor / LoadZone only; see §0) | `FUN_00472f90(1)` (unmasked) |
| 0x02 | level shutdown | `FUN_00472f90(2)` (unmasked) |
| 0x10 | **pre-tick**: once per frame, before objects move | `FUN_004a0375`, active list, activator 0 |
| 0x08 | **post-tick**: once per frame, after objects move | same, second loop |
| 0x40 | tag activated | `ActivateTag` (`FUN_00472b7c`): walls whose `i16 +0xa6` == tag, regions whose `i16 +0x86` == tag |
| 0x80 | "activate" (sent to an object named in an aux) | WTAGSW and others |
| 0x200 | wall touched | collision `FUN_004a4a1f`, activator = moving object (§1.1) |
| 0x400 | object landed on this region's floor | `FUN_004a41e3`, activator = object |
| 0x800 | object's head hit this region's ceiling | `FUN_004a4434` |
| 0x1000 | object entered this region | `FUN_004d8089` / `FUN_004d7fc6` |
| 0x2000 | object left this region | same |

Enter and leave events are sent only for the player (the activator is the current view's camera
object) and for network players (obj flag 0x80). They fire from `FUN_004d85a1` (re-home) when the
region changes.

**Pre-tick skip rule.** The pre-tick loop skips an entity whose byte3 has bit 2 set and whose +0x80
and +0x84 both equal the RLIMBO region (`DAT_005106f0`). For a wall, that means both sides are in
limbo. It also applies object-only region checks, which never trigger for walls or regions in the
data. (M)

**Frame order.** Pre-tick handlers run, then all objects move (and their collisions raise
0x200/0x400/0x800/0x1000/0x2000), then post-tick handlers run.

### 1.1 When a wall receives 0x200 (M)

This happens inside the object movement sub-step `FUN_004a4a1f`, for a wall within contact distance
(radius + 12 u; see physics.md §8). The activator is the moving object. There are two paths:

- **Solid branch.** It is taken when the wall is solid (flag 0x2) or the mover has obj flag 0x40.
  If the wall has flag **0x1** and tag `i16 +0xa6 ≥ 0`, the event is sent at once (asm 0x4a829b).
- **Common blocked exit** (`LAB_004a9387`). It is reached when a solid contact is resolved, or when
  a portal does not let the mover through. That includes the case where the mover is still on the
  near side of a passable portal line, so passable trigger lines fire on approach.

Both paths send 0x200 only if the wall differs from `DAT_00515538`, the last wall touched. After a
player (local or network) touches it, wall byte1 bit 0x40 is set. `DAT_00515538` is cleared after
any update in which the mover touched no wall. So a wall fires once per continuous contact.

Data: of the 615 WTAGSW walls,
- 113 are solid with flag 0x1 and tag ≥ 0;
- 201 are solid without flag 0x1 (only the common exit applies);
- 292 are passable portals.

### 1.2 Activator filters (H)

Several handlers check the activator before they act:

- `IsPlayer(o)` (`FUN_00489ff8`): `o` is the local player (byte2&4), a network player (byte0&0x80),
  or has byte 0x12 bit 0x20.
- `PlayerOrPlayersMissile(o)` (`FUN_0048a05a`): `IsPlayer(o)`, or `o` is a projectile-like object
  (byte2&0x20 or byte0x10&0x80) whose owner (`o+0x144`) passes `IsPlayer`.
- `FUN_0048a0df`: the same with owner test byte2&2.
- `AuxFilter(code)` (`FUN_0042426a`): a decimal-digit mask taken from aux+0x98.
  - **units digit**, checked on the activator itself:
    - 1 = local player
    - 2 = network player
    - 4 = other actor (byte3&1, not a player)
  - **tens digit**: the same three tests applied to the owner of a projectile (byte2&0x20).
  - **hundreds digit**: object byte 0x10 bits 4/8/0x10.
  - **thousands digit**: team checks in team game modes.
  - code 0 → any player-ish object.

### 1.3 Common region tag helper `RegionAuxTags` (`FUN_0044e6d0`) (H)

Used by RANIM, RFUP1, RFUP2 and RMOVE after their own work:

```
if activator && region.aux && (activator.state(+4) & 0x0f) == 0:
    ok = aux.p[0x98] > 0 ? AuxFilter(aux.p[0x98]) : PlayerOrPlayersMissile(activator)
    if ok:
        0x400  → ActivateTag(aux+0x90)     (landed on floor)
        0x800  → ActivateTag(aux+0x94)     (hit ceiling)
        0x1000 → ActivateTag(aux+0x88)     (entered)
        0x2000 → ActivateTag(aux+0x8c)     (left)
```

ActivateTag ignores tags ≤ 0.

### 1.4 Aux record fields used here

- Sound slots: aux+0x9c, +0xa0, +0xa4, +0xa8, +0xac. These are resolved from the name slots 0–4.
- Params: aux+0x88, +0x8c, +0x90, +0x94, +0x98 (i32).
- Name slots 5–9: char[12] at aux+0x4a, +0x56, +0x62, +0x6e, +0x7a.

**Sound calls.** `PlaySound(activator, snd, count)` (`FUN_0049d933` → `FUN_0049d6c7`).
`count < 1` means **loop**, `count = 1` means once. `StopSound(snd)` is `FUN_0049d489`.

**Moving-geometry sound convention:**

| slot | role | played |
|---|---|---|
| aux+0x9c | start | once |
| aux+0xa0 | moving | loop |
| aux+0xa4 | stop | once |

`FUN_0049e0bd` (preload aux sounds) is called by the init branches.

**Timers.** Wait deadlines use the 100 ms counter `Now100()` (`FUN_0049fc30`, `DAT_006cc47c`).
Shorter lockouts count down in animation units (see §4).

---

## 2. Wall effects

Wall runtime fields used by the handlers:

| wall offset | used as |
|---|---|
| +0xac | use count |
| +0xbc | switch state, or scroll speed |
| +0xc0 | lockout timer, or direction |
| +0xc4, +0xc8 | effect scratch |

These are "rare non-zero" bytes in the file. In the XRB they already hold the post-init values.

### 2.1 WTAGSW (index 206, flags 0x290, handler 0x45863a): switch / trigger line (H)

**Triggers.**
- 0x200 (wall touched): does the work.
- 0x10 (pre-tick): animates textures and runs the lockout timer.
- 0x80 is in the mask but has no branch. 0x40 is not in the mask, so tags do not toggle switches.

**Fields.**
- tag: `i16 +0xa6`
- state: +0xbc (0/1)
- lockout timer: +0xc0, 16.16 animation units
- limited uses: flag 0x1000 (wall byte1 bit 0x10) with the count in +0xac
- aux (optional), +0xdc:
  - +0x98: activator filter
  - +0x88: last tag of a tag range
  - slot 9 (+0x7a): name of an object to send 0x80 to

**Pseudocode:**

```
init (editor only):  state = 0; timer = 0; wall.byte1 |= 0x20

pre-tick:
    step every animated texture slot (upper/middle/lower) once (§4)
    if timer > 0: timer -= dt
    if timer <= 0: wall.byte1 &= ~0x40           // lockout released

wall touched (activator A):
    if wall.byte1 & 0x40: return                 // still locked out
    if A.state & 0x20: return
    code = aux ? aux.p98 : 0
    if code < 1: ok = PlayerOrPlayersMissile(A) || FUN_0048a0df(A)
    else:        ok = AuxFilter(code)
    if not ok: return
    if flags & 0x1000:                           // limited uses
        if uses < 1: return
        uses -= 1
    state ^= 1
    for slot in (upper, middle, lower): SetAction(slot, state)   // FUN_004a0c61, §4
    if aux and object named aux.slot9 exists: send it event 0x80
    if aux and aux.p88 > tag: ActivateTag(t) for t = tag .. aux.p88   (FUN_00472ac1)
    else:                     ActivateTag(tag)
    timer = 20.0                                 // 20 anim units = 240 ms at 12 ms/unit
```

The bump path also sets wall byte1 bit 0x40 right after the event. Together with the "last wall
touched" rule (§1.1), a switch therefore fires once per touch.

The switch **does not** reset itself. It toggles on every new touch, and each touch fires the tag
again. 16 walls have exactly 1 use (one-shot). 45 have no tag; touching them only toggles the
texture action.

Data:
- Only 1 of 615 walls has an aux, so the default filter applies: the player, or the player's missile.
- No shipped switch texture has a different action-1 start entry, so state changes are not visible.

`WOTAGSW` (0x45844c, unused) is similar. It fires only its own tag, accepts any activator unless
aux+0x98 sets a filter, and has the mask 0x4210.

### 2.2 WANIM (index 203, flags 0x10, handler 0x4593a0): animated wall (H)

**Triggers.** Pre-tick only.

**Behaviour.** Step the upper, middle and lower texture slots once per frame, with no "animated?"
check (§4). There is no state, aux or tag.

Data: 338 walls. 304 have at least one animated (Tex type bit 0) slot. Stepping a static Tex is
harmless.

### 2.3 WTEXHOR (index 201, flags 0x10, handler 0x458f17): horizontal texture scroll (H)

**Fields.**
- speed: +0xbc, 16.16 = `(aux+0x8c & 0xff) << 16`
- direction: +0xc0 = aux+0x88 (0 = +, non-zero = −)

**Pseudocode:**

```
init (editor only): speed = (aux.p8c & 0xff) << 16; dir = aux.p88
pre-tick (if aux):
    xoff(+0x9c) += dir == 0 ? Scale(speed) : -Scale(speed)
    xoff &= 0x00ffffff        // byte +0x9f cleared: offset wraps every 256 texels
    step all three texture slots (§4)
```

**Speed** = up to 255 texels per 1.2 s, about 212 texels/s.

Data: 7 walls, all in LIPS14, all with the texture TRNSIDE.
- 6 walls: speed 255.
- 1 wall: speed 127.
- Direction: 5 walls have dir = 1, which makes xoff decrease, so the texture scrolls toward −u.

`WTEXVER` (0x458e07) does the same with the y offset (+0xa0, byte +0xa3 cleared). It is unused.

### 2.4 WTV, the unnamed '&DAT' wall effect (index 202, flags 0x90, handler 0x4593f5): TV screen (M)

Its name string at `DAT_0051c862` is **"WTV"**. The neighbouring unnamed entries are WLEN (204),
WTOW (209), RTV (215), VDEF/VROT/VINS (views), and OPAL/ONEW/OEND/OKEY (objects).

**Behaviour.** The wall's middle texture shows the live picture of a secondary View, named by aux
slot 9 (TVVIEW, REMOTE, MIRROR1).

```
pre-tick (if aux):
    if not wall.flag 0x80000000:                    // first tick
        V = find View named aux.slot9; if none: return
        if V has no screen pic yet (its i16 +0x0c or +0x0e is -1):
            P = new Pic: rows = V.w(+0x10), cols = V.h(+0x12),
                log2 column stride = 6 / 7 / 8 for rows <= 64 / <= 128 / larger,
                pixels = V framebuffer (+0x14)
            saved = middle.pic; screen = P; view = V
            if wall has a middle texture: middle.pic = P
        V.flags bit 0x2 = (aux.p88 != 0)            // view on/off
        wall.flags |= 0x80000000
    if view:
        if V.flags & 2 == 0 and middle tex: middle.pic = saved; step slots   // TV off
        if V.flags & 2 and middle tex:      middle.pic = screen              // TV on
event 0x80: toggle V.flags bit 0x2
```

Event 0x80 cannot actually reach the handler: the initialised flag 0x80000000 is wall byte7 bit
0x80, which the masked dispatcher treats as "disabled". The view is therefore switched by whatever
sets the view's bit 0x2. That is probably OVIEWON/OVIEWOFF objects (not traced; L).

Data: 4 walls, all with aux+0x88 = 0, so the screen starts off and shows the normal texture. In
LIPS03 the view named MIRROR1 does not exist, so that wall stays a plain wall.

---

## 3. Region effects

**Region effect fields.**
- vars A–D: +0x6c, +0x70, +0x74, +0x78
- a 5th var E: +0x7c (not listed in XRB-records)
- per-region parameter: +0xa0 (i32)
- aux: +0x10c
- tag: `i16 +0x86`

**Current heights.** Floor +0x0c and ceiling +0x10. The handlers never read the rest heights
+0x14/+0x18, except RTAGSW, which reads +0x14.

**Textures.** Every region handler with a pre-tick branch first steps the floor slot (+0x1c) and the
ceiling slot (+0x34), but only if that slot's Tex has type bit 0 (animated). This is written
`AnimRegionTex` below.

**Carrying objects** (`FUN_0043b380(region, dz, flag)`). Used by every handler that moves a floor.
For each object in the region's object list:

- If dz < 0, the object weighs more than 10 (+0x40 > 10.0) and is not flying, its z moves down
  with the floor.
- If z ≤ floor, then z = floor.
- If z + height ≥ ceil, then z = ceil − height − 2.
- If the object still cannot fit and it is the local player (byte2&4), it is **not crushed**:
  - with flag = 1, the floor stops at ceil − height;
  - otherwise the ceiling is pushed to floor + height.

`TallestObject(region)` (`FUN_0044e052`) is the maximum Height (+0x34) over all objects in the
region list (0 if empty). Their positions are ignored.

### 3.1 RTDOOR (index 219, flags 0x3cd0, handler 0x451b77): rising door / ceiling (H)

**Triggers.**
- 0x40 (tag) or 0x80: start the motion.
- 0x10 (pre-tick): move.
- The mask also has 0x400–0x2000, but the handler ignores them. It does **not** call
  `RegionAuxTags`.

**Fields.**

| field | meaning | data |
|---|---|---|
| +0x10 ceil | moves; floor stays | stored = floor (closed) |
| +0x74 C | open ceiling height (absolute z) | = rest ceiling +0x18 in 156/156 |
| +0xa0 | signed speed, 16.16 u per 1.2 s. Sign = direction of the next move (+ open, − close) | 50 (67), 200 (61), 100 (15), 20 (7), 500 (4), 60, 1000; all positive |
| +0x78 D | state: 0 idle, 1 moving | 0 |
| +0x70 B | auto-restart deadline (Now100 ticks), 0 = none | 0 |
| region+4 bit 0x800000 (byte6 0x80) | "closed" flag: ceil == floor | set in 156/156 |
| aux+0x98 | wait before auto-close, × 100 ms | 50 (5 s) ×55, 30 (3 s) ×4, 0 ×82, no aux ×15 |
| aux sounds | +0x9c start, +0xa0 loop, +0xa4 stop | e.g. ELEV02A/AMBSFX02/AMBSFX06, DSLIDE02/–/– |

Open heights C − floor: 128 (113 doors), 64 (16), 256 (6), 144, 120, 32, ...

**Pseudocode:**

```
init (editor only):
    speed <<= 16;  C = ceil;  D = 0;  B = 0
    h = TallestObject;  ceil = h ? floor + h + 3 : floor          // start closed
    closedFlag = (ceil == floor)

tag 0x40 / event 0x80:
    if D == 0:
        D = 1; PlaySound(aux+0x9c, once); PlaySound(aux+0xa0, loop)
    closedFlag = (ceil == floor)
    // a tag while the door is already moving is ignored

pre-tick:
    AnimRegionTex
    if D == 0:
        if B != 0 and Now100() > B:                    // auto-close timer expired
            D = 1; play start + loop sounds
        return
    h = TallestObject
    if speed < 0 and h != 0:                           // closing onto objects
        if floor + h + 3 < ceil: ceil += Scale(speed)  // otherwise hold
    else:
        ceil += Scale(speed)
    if ceil >= C:                                      // fully open
        ceil = C; speed = -|speed|; D = 0
        if aux and aux.p98 > 0: B = Now100() + aux.p98
        StopSound(aux+0xa0); PlaySound(aux+0xa4, once)
    else if ceil <= floor + h:
        if h == 0:                                     // fully closed
            ceil = floor; speed = +|speed|; D = 0; B = 0
            StopSound(aux+0xa0); PlaySound(aux+0x9c, once)
        else:
            ceil = floor + h + 3                       // blocked: hover above the tallest object
    closedFlag = (ceil == floor)
```

**Behaviour summary.**
- **First tag opens.** The ceiling rises at `speed` up to C.
- **If aux+0x98 > 0**, the door closes on its own after that many × 100 ms. Otherwise it stays open
  until the next tag, which closes it (toggle).
- **Obstruction.** The door does **not** reverse. While any object is in the door region, the
  ceiling stops at floor + tallest height + 3 and keeps trying each frame. It finishes closing once
  the region is empty.
- **A tag during motion is ignored.**
- **No tag events of its own.**
- **Rendering.** The closed flag (region byte6 bit 0x80) makes the portal traversal in
  `FUN_00498009` skip the region, so a closed door hides what is behind it (M).

**Timing at 3 ticks per frame:** speed 200 → 5 u/frame (128 u in about 0.77 s). Speed 50 →
1.25 u/frame (128 u in about 3.1 s).

Data oddity: LIPS00 R408 has no tag, so it can never open.

`RTDOORM` (0x451fd1, unused) is a split door: floor and ceiling start at their midpoint, with the
speed in E (+0x7c).

### 3.2 RFUP1 (index 216, flags 0x3c10, handler 0x4512a5): perpetual lift / floor bob (H)

**Triggers.**
- 0x10: move.
- Afterwards it always calls `RegionAuxTags` for 0x400–0x2000.
- 0x40 is **not** in the mask. `RTFUP1` (0x451735, unused) is the tag-started one-trip version.

**Fields.**
- C (+0x74): bottom floor.
- speed: +0xa0, signed 16.16.
- D (+0x78): −1 = not started, 1 = going/at bottom, 2 = at top.
- B (+0x70): pause deadline.
- aux+0x98: pause in 100 ms units.
- aux+0x94: tag fired at the top (only if the player stands in the region).
- aux sounds +0x9c/+0xa0/+0xa4.

**Pseudocode:**

```
pre-tick:
    AnimRegionTex
    if D == -1 and aux: play start + loop; D = 1
    if B != 0:
        if Now100() <= B: return              // pausing (also skips RegionAuxTags)
        B = 0; play loop + start sounds
    h = TallestObject;  old = floor
    floor += Scale(speed)
    top = ceil - h - 2
    if floor >= top:                          // reached the top
        floor = top; speed = -|speed|
        if aux: StopSound(loop); PlaySound(stop); D = 2
        if player's region == this region: ActivateTag(aux+0x94)
        pause()
    else if floor <= C:                       // reached the bottom
        floor = C; speed = +|speed|
        if aux: StopSound(loop); PlaySound(stop); D = 1
        pause()
    CarryObjects(region, floor - old, 1)
  pause(): if aux and aux.p98 > 0: B = Now100() + aux.p98; StopSound(loop)
           else if aux: play loop + start again
```

**Data.** 9 regions: 7 in LIPS00, 2 in LIPS14.
- None has an aux, so there are no pauses and no sounds. The floor oscillates continuously between
  C and ceil − tallest object − 2.
- Speeds: 50 (6 regions), 100 (2), 200 (1).
- Example: R41 travels from −1824 up to about −770 (1054 u) at 41.7 u/s, which takes about 25 s
  each way.

`RFUP2` (0x450d86) is the same, except that the top is aux+0x90 (an integer, `<<16`) and it can
swap the floor texture to aux slot 8/9 names at the ends.

### 3.3 RFBOUNCE (index 211, flags 0x3c10, handler 0x452c9e): jump pad / trampoline (H)

**Fields.**
- region flag 0x200 (region byte1 bit 2): "jump pad". The physics code tests this bit.
- A (+0x6c): bounce speed = aux+0x88 << 16.
- B (+0x70): current bounce, starts at A.
- C (+0x74): mode = aux+0x8c.
- aux+0xac: bounce sound.
- aux+0x90 / +0x94: tags for landing / head bump.

**Handler.**

```
init (editor only): flags |= 0x200; A = B = aux.p88 << 16; C = aux.p8c
pre-tick: AnimRegionTex
0x400 (landed):  if activator is the local player (byte2&4) and not (activator.state & 0x80):
                     ActivateTag(aux+0x90)
0x800 (ceiling): if local player and not (activator byte5 & 2): ActivateTag(aux+0x94)
```

**The bounce itself is in the physics code** (physics.md §5.3, `FUN_004a4307`). It runs on every
sub-step with z == floor and vz ≤ 0 in a pad region:

```
if C == 1:                                    // decaying mode, only while jumping and jump held
    if obj jumped (byte6&2) and jump held: vz = B; B -= vz/4; if vz < A/4: vz = 0
else:
    vz = A                                    // mode 0: relaunch at A on every contact
if vz > 0: PlaySound(aux+0xac, once)
```

At jump start on a pad, the jump code also adds A to the launch speed.

**Data.** 40 regions, all mode 0, so the player bounces forever. The landing and head-bump tags are
all 0.
- Bounce speeds A: 300 (13), 200 (12), 600 (10), 350 (2), 450, 500, 400.
- For comparison, a normal jump launches at 200, which peaks at about 48 u.
- Sound: BOING04 (39 regions), BOING02 (1).

### 3.4 RANIM (index 214, flags 0x3c10, handler 0x4501d3): animated floor/ceiling (H)

```
pre-tick: AnimRegionTex
then: RegionAuxTags(event, activator)
```

Data: 79 regions. 76 have an animated floor or ceiling, for example water and lava.

### 3.5 RTAGSW (index 223, flags 0x3c10, handler 0x44fe36): region trigger (H)

**Triggers.** 0x1000 enter, 0x2000 leave, 0x400 land, 0x800 ceiling, and 0x10. There is no 0x40.

**Fields.**
- +0xa0: use count (raw int; 0 = unlimited). Data: 0 ×471, 1 ×8, 50 ×7.
- A (+0x6c): uses left.
- aux+0x88, +0x8c, +0x90, +0x94: tags for enter, leave, land, ceiling.
- aux+0x98: activator code.

**Pseudocode:**

```
init (editor only): A = +0xa0
any event other than 1 / 0x10:
    if no aux: return
    code = aux.p98
    if 0 < code != 100000: if not AuxFilter(code): return
    if code == 0 or code == 100000: if not IsPlayer(activator): return
    if +0xa0 > 0: if A < 1: return; A -= 1     // counts every passing event, even if its tag is 0
    0x400 → ActivateTag(aux+0x90); 0x800 → aux+0x94; 0x1000 → aux+0x88; 0x2000 → aux+0x8c
pre-tick:
    AnimRegionTex
    if aux.p98 / 100000 == 1 and aux.p8c > 0 and not (region byte5 & 0x20)
       and restFloor(+0x14) < floor and region has no objects:
        ActivateTag(aux+0x8c)                 // repeat "leave" tag while raised and empty; unused
```

**Data.** 486 regions.
- Tags used: enter only 201; land only 163; enter+leave 63; no aux 35 (only animates); other
  combinations make up the rest.
- All have aux+0x98 = 0, so only the player triggers them.
- 208 RTAGSW regions also carry animated floors.

### 3.6 RROTATE (index 212, flags 0x3ec8, handler 0x44f3e9): rotating sector (H for logic, M for edge cases)

**What moves.** The region's **wall endpoints** (Points) and the objects standing on its floor.
Textures do not move. Endpoints are shared with neighbouring walls, so attached walls deform
along.

**Triggers.**
- **0x08 (post-tick)**: rotation.
- 0x40 / 0x80: start or stop.
- 0x10 is not in the mask, so the floor/ceiling animation is stepped in the 0x08 branch.

**Fields.**

| field | meaning | data |
|---|---|---|
| B (+0x70) | mode: 0 idle, 1 running, 2 stop requested; any other non-zero value = runs forever | 0 (tag doors), 1 or 39 (always spinning) |
| E (+0x7c, read as i16) | angle step **per frame** (not time-scaled), signed | ±8, ±10, ±12 |
| A (+0x6c) | angle turned in the current segment | 0 |
| C (+0x74), D (+0x78) | segment counters | 0 |
| +0xf8 | pivot Point | stored in all 13 |
| +0xfc | pivot Object (pivot = its point), optional | none in data |
| +0x64 | Region named by aux slot 8 (init) | – |
| aux+0x90 | segment length in angle units (absolute value; 0 = no segments) | 512 (90°), 576, 768, or 0 |
| aux+0x94 | number of segments per activation (0 = until stopped) | 1 |
| aux+0x98 | 1 = ping-pong (reverse after every segment) | 1 |
| aux sounds | +0x9c start / even segment end, +0xa0 loop, +0xa4 odd segment end / stop | ELEV02A/B/C |
| wall +0x14/+0x18 | saved original p1/p2 positions (copies) | 102 walls |
| region +0xdc..+0xf0 | saved bbox and centre | the "second box" in XRB-records |

**Pseudocode:**

```
init (editor only): flags2 |= 0x90; A = 0; B = aux.p88; C = D = 0; E = aux.p8c
                    find pivot (object named slot9, else point named slot9, else a new point at
                    the bbox centre); save the original geometry; if B == 1 start the loop sound
tag 0x40 / event 0x80:
    if B == 0: A = 0; C = 0; B = 1; PlaySound(aux+0x9c, once); PlaySound(aux+0xa0, loop)
    else:      B = 2                                   // stop at the next segment end
post-tick:
    AnimRegionTex
    if B != 0 and aux and E != 0:
        RotateRegion(region, pivot, E)                 // FUN_0044eefd
        A += E
        if aux.p90 != 0 and |A| >= |aux.p90|:          // segment done
            C += 1; D += 1
            if aux.p98 == 1:
                E = -E                                 // ping-pong
                if D even and no pivot object: RestoreGeometry()   // snap back, removes drift
            A = 0
            PlaySound(C even ? aux+0x9c : aux+0xa4, once)
            if B == 2: (restore if C even and no pivot object); B = 0; C = 0
                       StopSound(aux+0xa0); PlaySound(aux+0xa4, once)
        if (aux.p94 > 0 and C >= aux.p94) or (aux.p94 == 0 and B == 2):
            B = 0; C = 0; StopSound(aux+0xa0); PlaySound(aux+0xa4, once)
```

`RotateRegion(R, (px,py), a)`:
1. Rotate each wall endpoint of R once about the pivot (a per-point "done" bit at Point+4 bit 2
   prevents double moves): x' = px + dx·cos a − dy·sin a, y' = py + dy·cos a + dx·sin a, using the
   2048-entry sine/cosine tables (counter-clockwise for a > 0).
2. Rotate each object in R with z ≤ floor that is not itself attached (obj+0xc bit 0x20 clear).
   Add a to its yaw and wrap it into 0..2047.
3. Clear the done bits and recompute R's bbox and centre.

**Example: DOOR01 in LIPS02** (tag 28, speed 12, segment 512, 1 segment, ping-pong).
- Each tag swings the door 90° in about 43 frames (about 1.4 s at 30 fps), then stops.
- The next tag swings it back.
- The angle overshoots by up to |E| − 1 units (516 instead of 512). After every second segment the
  saved geometry is restored.
- The rotation rate depends on the frame rate, because it is applied per frame.

Untagged spinners (B = 1 or 39, no limit) turn continuously, at 10–12 units/frame. In LIPS14, R20
spins in the opposite direction (−12).

### 3.7 RMOVE (index 213, flags 0x3ed0, handler 0x44fcf3): sector carried by an object (H)

**What moves.** Wall endpoints and the objects on the floor, translated in x/y only. The region
follows a **carrier object** named by aux slot 9 (data: RAFTMAN1, a path follower). These are the
rafts.

**Links** (set by init, and present in the XRB):
- region+0xfc = carrier;
- carrier+0x100 = region; carrier+0x0c |= 0x20 ("I carry a region"); carrier+0x10 |= 0x40;
- every wall of the region: wall+0x88 = region and wall+4 |= 8. This explains XRB-records' "third
  Region ref, 12 walls" and "wall +4 rarely 8". Data: 4/4 walls in each of the 3 raft regions.

**Movement.** It is not in the handler.
- Whenever the carrier is displaced by (dx, dy) — physics sub-steps, path following
  (`FUN_00489b24`), scripted moves (`FUN_004336a1`) — the mover calls
  `TranslateRegion(region, dx, dy)` (`FUN_0044eb5b`). That moves every endpoint once, plus every
  object in the region with z ≤ floor and not attached, then recomputes the bbox.
- A carrier yaw change rotates the region about the carrier (`FUN_0044eefd`, from `FUN_00462d35`),
  unless region+4 bit 0x80 is set (the RROTATE ownership bit).

**Handler.**
- 0x10: AnimRegionTex.
- Then RegionAuxTags. In the data, aux+0x88/+0x8c = 69/70 (LIPS00), 74/75 (LIPS14), 36/37 (LIPS16):
  tags for boarding and leaving the raft, which drive the raft script.

LIPS00 R416 has RMOVE but no aux, so it does nothing.

### 3.8 RSLANT (index 228, flags 0, handler `FUN_00450470`): ramps (H)

The ramp is a **container region** S (named RAMPnn) that encloses several thin strip regions. Each
strip has a flat floor, stepped by 16 u in the data. The renderer draws the strips flat. Collision
uses the strips for region membership and step-up, and the ramp code then raises z to a linear
height.

**Fields of S.**

| field | meaning |
|---|---|
| region byte2 bit 0x10 | "slanted" (set by init) |
| A (+0x6c), B (+0x70) | \|dx\|, \|dy\| between the two edge walls. Exactly one is non-zero in all data, so the ramp runs along Y if A == 0 and along X if B == 0. |
| C (+0x74) | slope = rise / run (16.16) |
| E (+0x7c) | base edge Wall (aux slot 8 name, "WBOTnn"). The XRB holds a **stale editor pointer** here, so it is re-resolved after load: `FUN_0047f7a5` calls the handler with event 1, and because the slanted bit is already set, it only does E = wall named slot 8. |
| +0x114 | number of strips |
| aux slots 5/6 | names of the high and low reference regions (DTOPnn/DBOTnn) |
| aux slots 7/8 | top/bottom edge walls (WTOPnn/WBOTnn) |
| aux +0x88 | rise (16.16) |
| aux +0x8c | run (16.16) |
| aux +0x90 | low z (16.16) |
| aux +0x94 | high z (16.16) |

**Strip regions** have region byte6 bit 0x02 (flags2 0x20000) and `i16 +0x114` = index of S.
Init marks every region whose bbox lies inside S's bbox and whose floor/ceiling lie within S's.
Data: 3–15 strips per ramp.

**Init (editor), for reference.**
- low/high = floors of the slot-6/slot-5 regions. If either region is missing, use aux+0x90/+0x94
  `<<16`, provided 0x94 > 0x90.
- run = mean distance between the two edge walls' endpoints. A, B = |Δx|, |Δy| of the walls'
  midpoints.
- C = rise / run.

**Runtime** (`FUN_004a0e32`). Called for an object whose current region has byte6 bit 2:
- from player physics (`FUN_004a1cfe`, after the vertical step);
- at the end of every object update (`FUN_004a4a1f`, after the floor/ceiling clamp).

```
S = region[obj.region.i16(+0x114)]; base = S.E (wall)
d = (S.A == 0) ? |obj.y - base.p1.y| : (S.B == 0) ? |obj.x - base.p1.x| : undefined
zr = aux.p90 + C * d                   // 16.16 multiply
if obj.z < zr: obj.z = zr              // walk up the ramp
if obj.z > aux.p94 and not flying: obj.z = aux.p94   // never above the top height
```

Data: 11 ramps. LIPS01 RAMP02 rises 96 u over 160 u (slope 0.6). LIPS13 RAMP02 runs along X:
256 u for 128 u of rise.

The "never above top" clamp also pulls a jumping player down while it is over a ramp strip above
the high height. This is faithful to the code (M).

---

## 4. Texture animation (walls, regions, objects) (H)

### 4.1 Data

The animation data are the texture slot (TC, 24 B), the Tex sequence entries
`{u16 frame_base, u16 next, i32 delay 16.16, u32 runtime}`, the pic table and the mirror table
(XRB-records §5).

TC runtime fields:

| TC offset | meaning |
|---|---|
| +4 | current entry |
| +8 | current pic |
| +0xc | timer, 16.16 |
| +0x10 | extra delay added at each step (0 in data) |
| +0x14 | mirror byte |

The renderer draws `TC.pic` (walls, flats) and does not step anything itself.

### 4.2 Clock (H)

Each frame, timers count down by `dt = ticks * 100 / 120` (16.16, `DAT_005670ec`). `ticks` is the
number of 10 ms engine ticks this frame, clamped to 1..20.

**One delay unit is 12 ms** (TB/100, where TB = 1.2 s).

Shipped world delays (`--anim`):

| delay | 12 ms units | time |
|---|---|---|
| 5 | 5 | 60 ms |
| 20 | 20 | 240 ms |
| 30 | 30 | 360 ms |
| 130 | 130 | 1.56 s |
| 999 | 999 | about 12 s (effectively a hold) |

At 30 fps, dt = 2.5 units per frame.

### 4.3 Step functions

`FUN_004a0b62`, the world version, is used by every wall/region handler:

```
StepWorld(tc):
    if tc.tex == null: return
    if tc.entry == null: SetAction(tc, 0); return
    if tc.timer < 1:                                   // i.e. <= 0 in raw 16.16
        if entry.next != 98:                           // 98 = hold the current entry
            entry = tex.entries[entry.next]            // 99 is NOT special here
        timer += entry.delay + tc.extra
        tc.pic    = tex.pics[entry.frame_base]
        tc.mirror = tex.mirrors[entry.frame_base]
    timer -= dt
```

`FUN_004a09fe`, the object version, has one difference: when the current entry's next is **99**, it
marks "sequence finished" and moves on to the following entry (entry + 1).
- Finished is set as obj+0x20 byte |= 2.
- If the object is a missile-like object (byte2 & 0x10 or 0x20), the flag is set on its owner
  (+0x144) instead.

`SetAction(tc, k)` (`FUN_004a0c61`):
- entry = entries[tex.start[k]] (u16 at tex+0x24+2k);
- timer = entry.delay − dt;
- pic and mirror taken from entry.frame_base.

It is used by WTAGSW (k = switch state) and by object state changes (`FUN_004a0d29` also stores k at
obj+0x158).

`SeqDone(tc)` (`FUN_004a08dc`) returns:
- 1 if the current entry's next points back to itself (a one-entry loop = final);
- 98 if it is a hold;
- 0 otherwise.

### 4.4 Rules

- At most **one entry advance per frame**. With delays below dt, the animation runs at the frame
  rate.
- A step takes the new entry's delay plus `extra`. The pic shown is `pics[frame_base]` with **no
  rotation offset** for world textures. Objects add the view rotation at draw time (renderer.md).
- **Hold (98)**: the frame stays and the timer re-arms with the same delay.
- **End (99)**: only objects handle it. No world texture uses 98 or 99: all 503 entries point inside
  their sequence.
- **Mirror flags.** They are copied into TC+0x14, but no world texture in the data has a mirror bit
  set. For sprites, the mirror flag flips the pic horizontally (renderer.md). Whether walls and
  flats honour it was not traced (L), and nothing needs it.
- **Who steps.**
  - Walls and regions animate **only** when their effect handler steps them: WANIM, WTAGSW,
    WTEXHOR/WTEXVER, WBMASK/WPATCH/WSTAIR, WTV, and every region handler with a tick branch.
    Region handlers and WTAGSW step only slots whose Tex has type bit 0. WANIM and WTEXHOR step all
    three slots.
  - Data: 59 walls and 3 regions carry an animated Tex but no stepping effect. They show their
    stored pic and do not animate.
  - Objects: static objects (byte3&8 set, byte0xd&8, byte1&4 and byte5&0x10 clear) are stepped in
    the main update loop. Moving or AI objects are stepped from their own handlers.
- **Initial state.** The fix-up sets entry = entries[0], and the stored timer is stale.
  Reimplementation: start at entry 0 with timer = entry0.delay. The first visible frame is the
  compiled pic (+8).

---

## 5. Function index

| address | role |
|---|---|
| 0x4727f4 | register effect (name, handler, flags) |
| 0x47291d | add (effect, entity) to the active list if flags & 0x18 |
| 0x472a54 | masked dispatch (entity, event, activator) |
| 0x472b7c / 0x472ac1 | ActivateTag / ActivateTag range |
| 0x472f90 | broadcast init (1) / shutdown (2) to all handlers |
| 0x4a0375 | frame: pre-tick 0x10 list, object updates, post-tick 0x08 list |
| 0x4d85a1, 0x4d8089, 0x4d7fc6 | re-home object; region enter/leave events |
| 0x4a41e3 / 0x4a4434 / 0x4a4307 | landing (0x400) / ceiling (0x800) / bounce incl. jump pads |
| 0x4a0e32 | ramp height (RSLANT runtime) |
| 0x47f7a5 | post-load pass (re-runs RSLANT event 1) |
| 0x44e052 | tallest object in region |
| 0x43b380 | carry/clamp objects after a floor/ceiling move |
| 0x44e6d0 (+0x44e4ce, 0x44e5cf) | region aux tag helper |
| 0x44eb5b / 0x44eefd | translate / rotate region geometry |
| 0x44e16d, 0x44e825, 0x44e992 | RROTATE pivot lookup, save geometry, restore geometry |
| 0x42426a, 0x489ff8, 0x48a05a, 0x48a0df | activator filters |
| 0x49d933 / 0x49d489 / 0x49e0bd | play / stop / preload aux sounds |
| 0x49fc30 | 100 ms tick counter |
| 0x4a09fe, 0x4a0b62, 0x4a0c61, 0x4a08dc | texture stepping (object/world), set action, sequence done |
| 0x45863a | WTAGSW |
| 0x4593a0 | WANIM |
| 0x458f17 / 0x458e07 | WTEXHOR / WTEXVER |
| 0x4593f5 | WTV |
| 0x4583cb | step the 3 wall slots (animated only) |
| 0x451b77 | RTDOOR |
| 0x451fd1 | RTDOORM |
| 0x4512a5 / 0x450d86 / 0x451735 | RFUP1 / RFUP2 / RTFUP1 |
| 0x452c9e | RFBOUNCE |
| 0x4501d3 | RANIM |
| 0x44fe36 | RTAGSW |
| 0x44f3e9 | RROTATE |
| 0x44fcf3 | RMOVE |
| 0x450470 | RSLANT |

## 6. Corrections for XRB-records.md

**Regions**
- **+0x86** is the region **tag** (i16), and it is used. "0xffff = AddRegion" is only its default.
- **+0x7c** is a 5th effect variable E: RROTATE speed, RSLANT base wall (stale pointer).
- **+0xa0** is the effect parameter: a 16.16 speed for RTDOOR/RFUP*, a use count for RTAGSW.
- **+0x114** is the RSLANT strip → container index, or the strip count on the container.
- **+0xdc..+0xf0** holds RROTATE's saved bbox and centre.

**Walls**
- **+0x14/+0x18** are saved original endpoint copies (RROTATE).
- **+0x88** and **+4 bit 8** are the RMOVE links.
- **+0xbc..+0xc8** are effect runtime variables.

**XRB values are post-init (§0).**

## 7. Open questions

1. **Wall 0x200 paths (§1.1).** The "common blocked exit" was reached through several gotos. That
   portal lines fire on approach, and that solid walls without flag 0x1 fire through it, is
   inferred (M). Verify under Wine: touch a solid one-sided switch without flag 0x1 (for example
   LIPS00 W231, tag 500) and a passable trigger line.
2. **WTV.** What sets View flag bit 0x2 in practice (OVIEWON?). How the secondary view is rendered
   into the screen pic.
3. **Sound positioning.** Effect sounds pass the activator, which is 0 for ticks and tags. The
   sounds are probably non-positional; not traced.
4. **RSLANT clamp.** The `z > high → high` clamp while not flying is applied every update. Confirm
   in game that jumping on ramps behaves that way.
5. **Frame-rate dependence.** RROTATE (and WTEXHOR's xoff via the integer-truncated Scale) depend on
   the frame rate. Pick a fixed logic rate (30 Hz, 3 ticks) to match the original feel.
