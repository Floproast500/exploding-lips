# Game loop timing and player movement physics

Clean-room notes for the G.O.D.D. engine as used in `lips.exe`. Everything here is paraphrased as
algorithms and constants. Addresses are the evidence.
Units: world coordinates and speeds are **16.16 fixed**. "u" means one world unit (0x10000).
Angles use **2048 units per full circle** (512 = 90 deg). Yaw is stored as a short (`obj+0x24`), or
as 16.16 in the overlapping int `obj+0x22`.
Confidence: **H** = read directly from code, **M** = inferred from code plus data, **L** = guess.

---

## 1. Overview

Each rendered frame:

1. **Win32 main loop** (`FUN_004dc382`, decomp ~80150): PeekMessage/Dispatch, then
   `now = GetTickCount()`. The engine frame `FUN_004bb183` runs only when
   `now - lastFrameTime > msPerFrame` (`DAT_00515c78`, default 33). This is the frame-rate cap. (H)
2. **Input sampling** `FUN_0047bfb0` (decomp ~37330): clears the player's action flags,
   polls the keys (key-binding handlers set flag bits) and the mouse (`FUN_0047a0c7`), then copies
   the yaw/pitch amounts into the player object. (H)
3. **Engine tick** `FUN_00472267`: computes the elapsed time in 10 ms ticks, sets the time-scale
   factor, syncs the camera to the player (`FUN_004ab912`), and updates all objects (`FUN_004a0375`).
   (H)
4. **Per-object update** `FUN_004a4a1f`. It calls the physics routine to get this frame's
   displacement (dx,dy,dz,dyaw):
   - `FUN_004a1cfe` for player-controlled objects (`obj[2]&4` local player, or `obj[0]&0x80`
     network player).
   - `FUN_004a102a` for all other physical objects.
   - `FUN_0045a943` for path followers.

   It then moves the object in sub-steps, with collision against objects, walls/portals, floor and
   ceiling. (H)

The physics is **variable-timestep**. Velocities are stored per *engine time unit* and scaled each
frame by the elapsed ticks. Some vertical terms are applied **per frame** without time scaling (see
section 5). (H)

---

## 2. Game loop and timing

### 2.1 Tick clock (H)
- `FUN_0049f8bb` initialises the clock from `GetTickCount`.
- `FUN_0049fa1c(now)` is called whenever more than 10 ms have passed since the last sample. It
  stores `DAT_006cc474 = now` and advances several counters:
  - 10 ms ticks (`DAT_006cc470`)
  - 40 ms ticks (`DAT_006cc478`, read via `FUN_0049fc7c`)
  - 100 ms ticks (`DAT_006cc47c`)
  - seconds (`DAT_006cc46c`)
- Accessors `FUN_0049fbe4`, `FUN_0049fc30` and `FUN_0049fc7c` re-sample `GetTickCount` first.

### 2.2 Engine ticks per frame (H) — `FUN_00472267`
```
ticks = (lastSampleMs - prevEngineMs) / 10      // integer division
ticks = clamp(ticks, 1, 20)                     // DAT_005670e0 ("Eng Ticks" in debug HUD)
prevEngineMs = lastSampleMs
SetTimeScale(ticks)                             // FUN_00473107
```

### 2.3 Time scaling (H) — `FUN_00472605`, `FUN_00473107`, `FUN_00473195`
- Base constant `TB = 120.0` (`DAT_005670dc`; `FUN_00472605(0x780000)` is called at init,
  decomp ~66259).
- `SetTimeScale(t)`:
  - `k = t / TB` (16.16, `DAT_005670e8`)
  - `DAT_005670e4 = TB / t`
  - `DAT_005670ec = 100*t / TB`. This is the per-frame decrement used by many countdown timers in
    object structs (`obj+0x118` and others).
  - It also selects row `t-1` of a precomputed 20 x 1024 table (`DAT_00567570`, "FPSTable").
- `Scale(v)` (`FUN_00473195`) returns `v * t / 120`:
  - When `|v| < 1024.0`, the result is looked up in the table by the **integer part** of v, so the
    fraction is dropped.
  - When `|v| >= 1024.0`, it is a fixed multiply by k.
  - The sign is preserved.
- **Conclusion:** speeds are in *units per 120 ticks = 1.2 s* (real time,
  `ticks = elapsed_ms/10`). For a reimplementation: `step = v * dt_ms / 1200`.
  - Keep the 1..20 clamp, so dt is at most 200 ms.
  - Optionally floor `dt_ms` to 10 ms granularity and drop the fraction of v for |v| < 1024 to match
    exactly.

### 2.4 Frame cap / optimalframerate (H)
- `DAT_005670f0` is the target FPS and `DAT_00515c78 = 1000 / DAT_005670f0` is ms per frame.
- Defaults (`FUN_004ba977`, decomp ~66079) are 30 and 33.
- The main loop runs a frame only when `now - lastFrame > 33`. That gives about 29.4 fps maximum,
  usually 3 engine ticks per frame.
- INI `[startup] optimalframerate=N` (`FUN_0046fc5b`) stores `N<<16`, which makes ms/frame
  `1000/(N<<16) = 0`, so **any INI value removes the cap**. This looks like an original bug.
- The console command (decomp ~111711) stores the raw N, which works correctly.
- The shipped INIs do not set it, so the shipped behaviour is a **30 fps cap**.

### 2.5 DOUBLESPEED (H)
- `DOUBLESPEED` is just the debug name of action bit 3 (`obj+0x20 & 8`, the "doubleaction"/run
  key). See the action-bit name builder at decomp ~45260 (next to `FUN_0048d761`).
- `FUN_0045a445`, on the rising edge of the run bit, doubles `msx, msy, ax, ay`
  (`obj+0x7c,+0x80,+0x8c,+0x90`) and sets `obj+0x124 = 1`. When the bit is released it halves them
  back.
- The vertical and turn values are not affected.

### 2.6 Debug HUD strings (for reference)
- "Frames/sec" = `DAT_00515ce8`
- "Eng Ticks" = ticks
- "MaxFrames/sec"
- "OptimalFrames/sec" = `DAT_005670f0`
- "PlrSpeeds sx/sy/sz" = `obj+0xac/+0xb0/+0xb4`
- "msx/msy/msz" = `obj+0x7c/+0x80/+0x84`
- "ax/ay/az" = `obj+0x8c/+0x90/+0x94`

---

## 3. Input → action flags

### 3.1 Per-frame flag word (H)
`obj+0x20` is a 32-bit action mask for the player object (`DAT_00510778`). The input routine resets
it every frame (it keeps only bits 1 and 2) before the key handlers run. The bit names come from the
debug decoder (decomp ~45260):

| bit | mask | name | set by |
|---|---|---|---|
| 0 | 0x1 | ACTIVATETOOL | activatetool |
| 1 | 0x2 | SHOOTPROJECTILE | |
| 2 | 0x4 | TOOLDONESHOOT | |
| 3 | 0x8 | DOUBLESPEED (run) | doubleaction `FUN_00479164` |
| 4 | 0x10 | FORWARD | moveforward `FUN_00478d39` |
| 5 | 0x20 | BACKWARD | movebackward `FUN_00478e14` |
| 6 | 0x40 | LEFT (strafe) | moveleft `FUN_00478fa0`; turnleft in strafe mode |
| 7 | 0x80 | RIGHT (strafe) | moveright `FUN_00478ff6`; turnright in strafe mode |
| 8 | 0x100 | UP (jump / fly up) | moveup `FUN_00478ec8` |
| 9 | 0x200 | DOWN (crouch / fly down) | movedown `FUN_00478f5d` |
| 10 | 0x400 | TURNLEFT | turnleft `FUN_0047904c` |
| 11 | 0x800 | TURNRIGHT | turnright `FUN_004790b1` |
| 12 | 0x1000 | LOOKLEFT | lookleft `FUN_00478ccb` |
| 13 | 0x2000 | LOOKRIGHT | lookright `FUN_00478d02` |
| 14 | 0x4000 | LOOKUP | lookup `FUN_00478c3b` |
| 15 | 0x8000 | LOOKDOWN | lookdown `FUN_00478c83` |
| 16 | 0x10000 | CENTER VIEW (byte +0x22 bit0) | lookcenter `FUN_00478bee` |
| 18 | 0x40000 | fly-along-pitch up (+0x22 bit2) | forward/back while flying with "look-fly" on |
| 19 | 0x80000 | fly-along-pitch down (+0x22 bit3) | as above |
| 20 | 0x100000 | landing squat (+0x22 bit4) | set by the landing timer (section 5.6) |
| 28 | 0x10000000 | DESTROY | |

`obj+6 |= 0x40` marks "is moving" (used for animation). `obj+6 |= 2` is set by jump.

### 3.2 Key handler details (H)
- **moveforward**: sets FORWARD, or BACKWARD when `DAT_00510d84` is set. That flag is true while
  the *reverseaction* key is held (default VK 0x20, `DAT_00510d80=32`, polled with
  GetAsyncKeyState).
- **movebackward**: sets BACKWARD, or FORWARD when the reverse key is held.
- **forward/back while flying**: if the player flies (`obj[0]&2`), the ext struct has "look-fly"
  enabled (`ext+0xc & 2`, console toggle "ON"/"1", decomp ~111691), and pitch ≠ 0, the handler also
  sets the fly-along-pitch up/down bit according to the pitch sign.
- **moveup**:
  - Sets UP and `obj+6|=2`.
  - If VK_SHIFT is held (`DAT_00510d7c=16`) it also sets FORWARD (running jump).
  - Raises event tag 0x3b3.
  - If not flying, sets `DAT_00510ce0=1` ("this airborne phase began with a jump").
- **movedown**: sets DOWN and raises tag 0x3ad.
- **doubleaction**: sets the run bit and raises tag 0x3ae.
- **turnleft / turnright**:
  - Normal mode: sets TURNLEFT/TURNRIGHT and yaw amount `in+0x54 = +kbLRsensitivity` (or `-`).
  - When strafe mode `DAT_00510da4==1` (blimp style): sets LEFT/RIGHT instead.
- **lookup / lookdown**: pitch amount `in+0x58 = mousepitch`, then set LOOKUP/LOOKDOWN.
- **lookcenter**: sets bit 16 (`obj+0x22|=1`).
- The `in` struct is `DAT_005931dc`:
  - `+0x18` kbLRsensitivity, `+0x1c` mouseyaw, `+0x20` mousepitch. Defaults are **24** each
    (decomp ~65636). The menu clamps them to 2..49.
  - `+0x54` yaw amount, `+0x58` pitch amount for this frame.
  - `+0x34` mouse axis-lock threshold.
  - `+0` bit0 = invert mouse Y.
- At the end of input processing (decomp ~37403) the amounts are copied into the player: short
  `obj+0x1c8 = in+0x54` (yaw) and short `obj+0x1ca = in+0x58` (pitch).

### 3.3 Mouse (H) — `FUN_0047a0c7(obj, dx, dy, b1, b2, b3, altHeld)`
1. Reset the yaw and pitch amounts to 0.
2. Axis lock: if `|dx| - |dy| > threshold` (`in+0x34`), zero the smaller axis.
3. **X axis**:
   - If Alt is not held and strafe mode is off, X turns:
     - yaw amount = `|dx|` when `|dx| <= mouseyaw`, else `(|dx|/35 + 1) * mouseyaw`.
     - dx < 0 → positive amount + TURNLEFT. dx > 0 → negative amount + TURNRIGHT.
   - Otherwise X strafes: sets LEFT or RIGHT.
4. **Y axis**:
   - pitch amount = `|dy|` when `|dy| < mousepitch`, else `(|dy|/30 + 1) * mousepitch`.
   - Normal mode: dy < 0 → LOOKDOWN, dy > 0 → LOOKUP. The invert bit swaps them.
   - Strafe mode: dy < 0 → FORWARD, dy > 0 → BACKWARD.
5. Mouse buttons go to `FUN_00479876`.

Mouse deltas are therefore **per-frame angle amounts**. They are not scaled by time.

---

## 4. Horizontal movement — `FUN_004a1cfe` (player physics)

Per-object fields:
- `vx` = `obj+0xac`: forward speed along facing.
- `vy` = `obj+0xb0`: strafe speed, positive = left.
- `vz` = `obj+0xb4`: vertical speed.
- `vt` = `obj+0xb8`: turn, used only as a "turning" indicator.
- Maximums: `msx/msy/msz/mst` = `+0x7c/+0x80/+0x84/+0x88`.
- Accelerations: `ax/ay/az` = `+0x8c/+0x90/+0x94`.

Movement model `M = DAT_00510774`:
- Zero-initialised, so **M=0 by default**.
- `OPHYSON` sets it from the effect object's `+0x54`, `OPHYSOFF` sets it to 0, and console commands
  `FUN_00409fd8/FUN_0040a025` also set it.
- No shipped level references OPHYSON by name, so **assume M=0** (confidence M).

Friction/gravity vector `F = (fx, fy, g)`, chosen in this order:
1. The ext struct's `+0x230` block if `ext+0xd & 0x10`.
2. Otherwise the current region's `+0x90` block if `region[1] & 1` (per-region physics such as ice
   or water).
3. Otherwise the level globals `DAT_005676cc/d0/d4`.

The level globals come from the XRB **Tail** block (0x7c bytes read to `DAT_005676ac`, decomp
~75316):

| field | offset in Tail | meaning |
|---|---|---|
| `DAT_005676cc` | +0x20 | fx |
| `DAT_005676d0` | +0x24 | fy |
| `DAT_005676d4` | +0x28 | g |
| `DAT_005676d8` | +0x2c | angle (degrees*512/90), used by non-player physics |

All levels checked (LIPSTART, 00, 01, 03, 05, 10) have **fx=200, fy=200, g=400**. The angle is 568
(=100 deg), or 1137 (=200 deg) in LIPS00. `OSETGEN` (`FUN_00432674`, args fx,fy,g,deg) and a
console command (`FUN_0040c77d`) can override them.

```
// start of frame
if M==0: vx = vy = 0          // no inertia at all
if M==1: vy = 0               // strafe has no inertia, forward does
vt = 0
// look keys (section 7) are handled here
if FORWARD:  vx = (M==2 && !UP) ? min(vx + Scale(ax), msx) : msx
if BACKWARD: vx = (M==2)        ? max(vx - Scale(ax), -msx) : -msx
if LEFT:     vy = (M==2) ? min(vy + Scale(ay),  msy) :  msy
if RIGHT:    vy = (M==2) ? max(vy - Scale(ay), -msy) : -msy
if TURNLEFT:  vt = +mst
if TURNRIGHT: vt = -mst
// vertical: section 5
// yaw (section 6) is applied here
// friction, only when M != 0 and not obj[0]&4:
if vx<0 && !BACKWARD: vx /= 2; fricX = -fx      // arithmetic shift
if vx>0 && !FORWARD:  vx /= 2; fricX = +fx
if M==2: fricY = sign(vy)*fy
// apply friction, halved while airborne (z != floor). Clamp to 0 if it would cross zero.
vx -= Scale(onFloor ? fricX : fricX/2)
vy -= Scale(onFloor ? fricY : fricY/2)
// displacement for this frame
f = Scale(vx); s = Scale(vy)
dx =  cos(a)*f - sin(a)*s
dy =  sin(a)*f + cos(a)*s
dz =  Scale(vz)
```
- `a` is the yaw. The code evaluates `sin(-a)` and `cos(-a)` with FUN_00468c7f/FUN_00468cc2. The
  result is a right-handed frame: at a=0 forward is +x and left is +y. a=512 faces +y. Turning left
  increases a. (H; handedness confirmed against the renderer: +y is left of +x, CCW angles.)
- **Player defaults** (LIPS00 start object #715, Objects record):
  - msx = msy = **800**: walk is 800 u per 1.2 s ≈ 667 u/s. At 3 ticks/frame that is 20 u/frame.
  - Run doubles this to 1600.
  - msz = **200**.
  - mst = **853** (=150 deg in the degree*512/90 encoding).
  - ax = ay = az = **400**.
  - `+0x98 = 2275` (=400 deg, probably turn acceleration, unused by player code).
  - `playerspeedmaxx/y/z/t` INI keys override `+0x7c..+0x88` when they are not -1
    (`FUN_0048f234`; the deltas are applied to net players by decomp ~13554). (H for the data
    values)
- Turning (`vt`) magnitude is ignored for the player. See section 6.

---

## 5. Vertical: gravity, jump, fall, crouch, fly

### 5.1 Player extension struct `ext = obj+0x1b8` (H)
The local player gets the static block `0x5b3028` (`FUN_0048d123`). Network players get a copy
(decomp ~44895). Default field values (msz = 200 for the player):

| ext+ | default | meaning | set by |
|---|---|---|---|
| 0x00c | flags | 0x02 look-fly, 0x04 jump apex passed, 0x08 crouched, 0x10 jumping, 0x20 fly-up anim, 0x40 fly-down anim, 0x80 falling | |
| 0x00d | flags | 0x01/0x02 fly anim, 0x08 high fall, 0x10 use ext+0x230 physics | |
| 0x204 | msz (200) | fly up speed | OFLYON arg1, OSETZSPD arg1 |
| 0x208 | msz | fly down speed | OFLYON/OSETZSPD arg2 |
| 0x20c | msz/4 (50) | fly idle sink speed | arg3 |
| 0x210 | msz/4 | fly speed along pitch | |
| 0x214 | msz | fall speed: per-frame fall accel is this/4, and the speed is snapped to −this at the apex | OFLYON/OSETZSPD arg4 |
| 0x218 | 0 | extra dz on the jump frame | OSETJUMP arg0 |
| 0x21c | msz (200) | jump launch speed | OSETJUMP arg1 |
| 0x220 | 2.0 | ? (unused in the player path) | OSETJUMP arg2 |
| 0x224 | msz | gravity while jump-falling (per frame this/4) | OSETJUMP arg3 |
| 0x228, 0x258 | 0 | landing squat divisor | OSETJUMP arg4 (raw int) |
| 0x22c | 0 | active OFLYON/OSETZSPD effect object (sounds) | |
| 0x240/0x244 | | jump start z / jump ceiling z | |
| 0x248 | 0 | active OSETJUMP effect object (sounds GRUNT03, -1, GRUNT01, BREAK01 = def+0x9c..0xa8) | |
| 0x24c | 4 | crouch height divisor | OSETCROU arg0 |
| 0x250 | 0 | fixed crouch height (if >0) | OSETCROU arg1 |
| 0x254 | 500 | crouch/stand speed | OSETCROU arg2 |
| 0x25c | 500 | ? | OSETCROU arg4 |

Effect arguments are stored in the effect object's definition at `def+0x88, +0x8c, +0x90, +0x94,
+0x98` (arg0..arg4). They are converted with `<<16` unless noted.
**`OSETJUMP=0,200,0,200,6,GRUNT03,-1,GRUNT01,BREAK01`** (all levels except LIPS03, which uses 250)
gives:
- hop 0
- launch 200
- (2.0 → 0)
- jump-fall gravity 200 (→ 50 per frame)
- landing squat divisor 6
- sounds: jump=GRUNT03, apex=none, land-hard=GRUNT01?, landing=BREAK01

(Sound slot order: M.)

### 5.2 Walking mode vertical update (not flying), in order (H)
`floor`/`ceil` are the current region's `+0xc`/`+0x10`. `z` is `obj+0x2c` (the feet). `G` is F.g.
`step` is `DAT_005670f4` = **32 u** (reset to 32 at level load, decomp ~33074; `OENGINE` arg0
overrides it).

1. **Jump start**: on UP (or fly-pitch-up) **and `z == floor` exactly**:
   - `ext.jumpStart = z`.
   - `ext.jumpTop = z + (jumpEffectObj+0x84 > 0 ? that : 2048 u)`. This is effectively unlimited.
   - `vz = ext+0x21c` (200). If `ext+0x218 ≠ 0`, the frame's dz is forced to it.
   - Play the jump sound and clear the apex flag.
   - Jump-pad region (`region[1]&2`): `vz += region+0x6c` (also remembered in `region+0x70`).
   - Set `ext.jumping` (0x10).
   - Without an ext struct: `vz = msz`.
2. Crouch / stand height (5.4).
3. If `vz > 0`: `gAcc = G` (else 0, in M=0).
4. If `vz < 1` (not rising):
   - **Airborne**: `z - floor > step` or the region has no floor (`region[0]&0x10`).
     - Not jumping: `vz -= ext+0x214/4` **per frame**.
       - The first time: set the falling flag (0x80), store the fall start `DAT_00515534 = z`, and
         set the high-fall flag if `z - floor >= freefallhighdist` (128 u).
     - Jumping: `vz -= ext+0x224/4` **per frame**.
       - The first time: set the apex flag (0x04), play the apex sound, store the fall start, and
         check high fall.
     - High-fall flag set: additionally `vz -= az/2` per frame (az=400 → 200).
     - No ext: `vz = -G`.
   - **Within step height of the floor**: clear the falling flag. If not jumping (or vz == 0),
     **snap `z = floor`**. This is how step-down of ≤32 u works.
5. Else (rising): if `pos.z >= ext.jumpTop`, then vz = 0 and the apex flag is set.
6. If not falling-flagged: `vz -= Scale(gAcc)`. If vz was ≥0 at frame start and is now <0, snap
   `vz = -(ext+0x214)` (= −200).
7. `dz = Scale(vz)`. Then `obj[0xf]&0x20` (frozen) forces vz = dz = 0.

Worked example (30 fps cap, 3 ticks/frame, Scale = v/40):
- **Jump**: vz drops 10/frame from 200 over about 20 frames (0.68 s). Peak height ≈ **47-50 u**,
  roughly one player height.
- **Fall**: starts at −200 and gains −50 per frame (frame-rate dependent!). Falling 128 u takes
  about 11 frames.
- Confidence: H for the formulas, M for the time interpretation.

### 5.3 Landing (H) — sub-step loop in `FUN_004a4a1f`
- `FUN_004a41e3`: if z dips below the floor, set `z = floor`. In fly mode a negative vz is
  reflected. Then `vz < 12 u → 0`. The object's "hit floor" event 0x400 fires once per landing.
- `FUN_004a4307` (on the floor with vz ≤ 0):
  - Normal regions: walking `vz = -vz/2` (half bounce), flying `vz = -vz`, then `<12 → 0`.
  - Jump-pad regions (`region[1]&2`):
    - mode `region+0x74 == 1` with the jump key held: decaying bounce. `vz = region+0x70`, and
      region+0x70 is reduced by vz/4 each bounce until it drops below region+0x6c/4.
    - other modes: always `vz = region+0x6c`.
- On the floor with `0 < vz < 12 u`: vz = 0.

### 5.4 Crouch (H)
- `Height` = `obj+0x34` (current), `FullHeight` = `obj+0x38`. The player has **48 u** for both.
- DOWN held: target = `FullHeight / ext+0x24c` (48/4 = **12 u**), or `ext+0x250` if that is > 0.
  - If `ext+0x254 > 0`, Height decreases by `Scale(ext+0x254)/divisor` per frame (500 →
    3.125 u/frame at 3 ticks, about 12 frames).
  - Otherwise Height jumps straight to the target.
  - Sets `ext.crouched` (0x08).
- Released: Height increases at the same rate back to FullHeight. It is clamped so
  `Height <= ceil - floor - 8 u` (you stay crouched under low ceilings).
- No ext: Height = FullHeight/4 instantly.
- A region whose `ceil - floor < Height` cannot be entered (portal check in 8.2).
- The camera height follows Height (section 7), so crouching lowers the eye from 48 to 12 u.

### 5.5 Fly mode (`obj[0] & 2`) (H)
- Enter: `OFLYON` (`FUN_00429d26`). It sets the flag, zeroes vz, and sets `Height = arg0` (raw
  copy, no `<<16`; units unclear, L). Args 1-4 → `ext+0x204/208/20c/214`.
- `OFLYOFF` leaves fly mode.
- Each frame vz is reset to 0, then:
  - UP: vz = ext+0x204 (200).
  - DOWN: vz = −ext+0x208.
  - Fly-pitch bits: ±ext+0x210 (50).
  - No ext: msz, or msz/4 for the pitch bits.
  - If vz == 0 and (`z - floor > 4 u` or no floor): vz = −ext+0x20c (idle sink, 50).
  - **maxflyheight** (`DAT_00567100`, INI or OENGINE arg3): if it is > 0, vz > 0 and the
    **absolute** z ≥ max, then vz = 0.
- Look-fly (`ext+0xc&2`) with pitch ≠ 0 and vx ≠ 0:
  - `d = vx * |pitch| / 520` (pitch from `obj+0x28`, range ±512).
  - `vx -= d`, and `vz ±= d` with the sign of the pitch. Forward motion is redirected along the
    view pitch.
- Gravity and the airborne logic of 5.2 are skipped. Friction in M=2 uses ±g/2 as vertical drag.

### 5.6 Falling damage / hard landing (M)
In the player object's event handler `FUN_0045bbe0` (OPLAY), "hit floor" event 0x400 (decomp ~129550):
- Fall distance `D = fallStartZ - z`.
- Walked off a ledge (`DAT_00510ce0 == 0`): if `D > 96 u` (`DAT_00515530`) and
  `D >= freefallhighdist` (128 u, `DAT_0056710c`, INI `freefallhighdist`) and the region is not a
  jump pad, it schedules a **landing squat**: `DAT_00510798 = now40ms + 2`.
- Jumped (`DAT_00510ce0 == 1`): the squat is scheduled only if the high-fall flag was set. The
  jumping flag is cleared.
- If `D >= 128 u`, it plays the OSETJUMP landing sound (def+0xa8, "BREAK01"), or else raises event
  tag 0x3e6.
- It then resets the fall start to −9999 u and clears the jump/apex/fall/crouch flags
  (`ext+0xc &= 0xf76b`).
- About 80 ms later the input routine sets the squat bit (`obj+0x22|=0x10`, `FUN_0045bbe0` decomp ~129778). That
  frame Height drops by `FullHeight / ext+0x258` (48/6 = 8 u). It then recovers at crouch speed.
- **No hit-point damage was found in this path.** Damage, if any, would come from the 0x3e6 tag
  being mapped in script, and no shipped INI maps it. (M)

---

## 6. Turning (H)

The turn is applied in `FUN_004a1cfe` after the vertical step (address 0x4a310e…0x4a31c3):
```
if vt != 0:                                  // TURNLEFT/TURNRIGHT was pressed this frame
    if isPlayer && short(obj+0x1c8) != 0:
        a += short(obj+0x1c8); obj+0x1c8 = 0  // kb: ±kbLRsensitivity, mouse: computed amount
    else:
        a += (vt > 0 ? +20 : -20)
a &= 2047
```
- Turning is **per frame and not time-scaled**. The default keyboard rate is 24 units/frame
  (4.2 deg/frame, about 124 deg/s at 29 fps).
- `mst` (853) only sets the sign of `vt`.
- The code calls `Scale()` on a local that is always 0, so there is no turn decay. This looks like a
  leftover.
- The returned `dyaw` (param_3[3]) is the applied delta.
- Camera yaw = `-(view.lookLR + a)` (`FUN_004ab912`).

---

## 7. View / look (H)

`View = DAT_005675a0` (or `DAT_0051098c` when set). Fields:
- `+0x2c` attached object
- `+0x34` H, the eye z
- `+0x38` visibility distance
- `+0x3e` short look-left/right offset
- `+0x40` short pitch
- `+0x42` short derived y-shear
- `+0x80` int scale used for the shear

Behaviour:
- **Eye height**: each frame `view.H = obj.RH = z + Height` (`FUN_004a0375`, decomp ~55046;
  `FUN_004d848b` keeps `RH = z + Height`). Standing eye = floor + 48 u, crouched = floor + 12 u.
  There is no separate eye offset.
- **Look up/down**:
  - `pitch += short(obj+0x1ca)` per frame while LOOKUP is held (`FUN_0047bad1`), and `-=` for
    LOOKDOWN (`FUN_0047bb61`). Default 24/frame.
  - Clamped by `FUN_004ac49f` to `[DAT_00567114, DAT_00567110]` = **[−512, +512]**. Defaults are
    set at decomp ~66088. OSETVIEW args 0/1 and a console command override them.
  - The same function sets `y-shear = ((pitch + 127) * (view+0x80 >> 16)) >> 8` (127 =
    `DAT_00567118`) and copies the pitch to `obj+0x28` (used by look-fly).
- **Look left/right**: `lookLR ±= 16` per frame (`FUN_0047ba30`/`FUN_0047b993`). The new value is
  rejected if it would exceed ±512.
- **Center view**: the lookcenter bit makes the next player physics tick call `FUN_0047bbf1`, which
  sets lookLR = pitch = 0 **instantly**.
- FOV is `DAT_0056711c`. The camera ray setup uses ±FOV/2 around the yaw (decomp ~55030).

---

## 8. Collision and moving through regions — `FUN_004a4a1f` (H unless noted)

### 8.1 Sub-stepping
- If `max(|dx|,|dy|) <= 12 u`: one step.
- Otherwise `N = floor(max/1u)/8 + 1` steps of `d/N`, so each step is at most about 8 u.
- Each step:
  1. Move (x,y,z) by one step.
  2. **Floor/ceiling**:
     - `z < floor` → land (`FUN_004a41e3`).
     - `z == floor && vz <= 0` → bounce (`FUN_004a4307`).
     - `ceil <= z + Height + 8 u` → ceiling hit (`FUN_004a4434`), which zeroes dz.
  3. **Objects**: for each solid object in the region (`region+0xb4` list) with vertical overlap:
     - Collide if `|dx|, |dy| < r1 + r2` (radius `obj+0x3c`; **player r = 16 u**).
     - Then the exact distance is computed with an integer sqrt (`FUN_00468fac`).
     - If neither object has `obj[0]&1`, push this object out along the centre line by the overlap.
     - Push the other object by a weight ratio (`obj+0x40`) if it is movable.
     - Fire bump events 0x200.
  4. **Walls** of the region (`region+0xb8` array, count `+0xbc`). For each wall segment p1→p2:
     - Quick reject: the bounding box expanded by `r + 16 u`.
     - Compute the wall angle (atan2), the along-wall coordinate `t` and the signed perpendicular
       distance `d` of the new position.
     - Contact if `|d| < r + 12 u`. For "thick" walls (`wall[2]&2`) the threshold is
       `max(r + 12 u, 40 u)`.
     - **Portal (two-sided, not solid `wall[0]&2`)**: see 8.2.
     - **Solid wall**: fire event 0x200 on the wall (switches, etc.). Then:
       - `0 ≤ t ≤ len`: set the position to `p1 + t*dir + n*(r+12 u)` on the side the object came
         from. The perpendicular component is removed and the along-wall motion is kept, so the
         object **slides along the wall**.
       - `t < 0` or `t > len`: push radially away from the nearest endpoint to distance `r+12 u`
         (rounded corners).
       - If the wall is movable (`wall+0x90` weight), part of the push is transferred to the wall.
     - If the push would land outside any region, or in a region too narrow for the radius, the
       whole step is undone.
- After the loop:
  - Apply the floor-ramp offset (8.2).
  - Clamp `z ≥ floor` (unless the region has no floor).
  - If `z + Height > ceil`, set `z = ceil - Height - 8 u` (unless the region has no ceiling).
  - Write back the position.
  - Clamp x,y inside the region bounding box (`region+0xc4..+0xd0`) with an 8 u margin.

### 8.2 Portals and step-up (M/H)
- A two-sided wall links `wall+0x80` and `wall+0x84` regions.
- The opening is bounded by the higher of the two floors and the lower of the two ceilings. Floor
  and ceiling link regions (`region+0x5c/+0x60`, flags 0x10/0x20) can replace these.
- You can pass if all of these hold:
  - `nextFloor - step(32 u) <= z`, i.e. **step-up ≤ 32 u**.
  - `z + Height < nextCeiling`.
  - `nextCeil - nextFloor >= Height`.
  - The wall is not impassable (`wall[1]&2` clear).
  - Mask/flag checks pass (`obj+6&8` confines the object to its region; `obj+6&0x10` uses a region
    group mask).
- On crossing (sign of d flips past the line): re-home the object to the new region
  (`FUN_004d85a1`) and raise `z` to the new floor if it is lower.
- **Floor ramp**: when within `max(r, 32 u)` of a portal whose other floor differs, an offset
  `e4` linearly interpolates between the two floor heights by distance to the line. It is limited
  to the actual difference. Stepping up or down is therefore smoothed rather than instant. (M — the
  formula has two variants depending on floor < −1024 u.)
- If the step is blocked, the wall is treated as solid (slide).

### 8.3 walldistinside / walldistoutside are NOT collision (H)
- `DAT_005670f8/fc` are copied to `DAT_006c6d64` depending on the indoor/outdoor state
  (`DAT_00515d38`).
- That value is a **draw/visibility distance** for walls and objects (renderer decomp ~6279 and
  ~51356).
- The collision distance is always `radius + 12 u`.
- `OENGINE` (`FUN_00430f25`) args: stepheight, walldistinside, walldistoutside, maxflyheight, arg4
  (`DAT_00567104`, a toggle on render flags).

---

## 9. Constants table

| constant | value | units | source | conf |
|---|---|---|---|---|
| engine tick | 10 ms | | `FUN_00472267` | H |
| ticks per frame clamp | 1..20 | ticks | same | H |
| time base TB | 120 ticks (1.2 s) | | `FUN_00472605(0x780000)` | H |
| frame cap | 33 ms (30 fps) | | `DAT_00515c78`, main loop | H |
| player walk msx/msy | 800 | u / 1.2 s | XRB obj #715 +0x7c/+0x80 | H |
| run multiplier | ×2 (msx,msy,ax,ay) | | `FUN_0045a445` | H |
| msz | 200 | u / 1.2 s | +0x84 | H |
| mst | 853 (150°) | angle/1.2 s (sign only) | +0x88 | H |
| ax/ay/az | 400 | u/1.2 s per 1.2 s | +0x8c.. | H |
| friction fx/fy | 200/200 | u/1.2 s per 1.2 s | XRB tail +0x20/+0x24 | H |
| gravity g | 400 | u/1.2 s per 1.2 s | XRB tail +0x28 | H |
| jump launch | 200 (LIPS03: 250) | u/1.2 s | OSETJUMP arg1 | H |
| jump-fall accel | 200/4 = 50 per frame | u/1.2 s | OSETJUMP arg3 | H |
| fall accel / apex snap | 200/4 per frame / −200 | | ext+0x214 = msz | H |
| high-fall extra accel | az/2 = 200 per frame | | | H |
| step height | 32 u | | `DAT_005670f4` | H |
| player radius | 16 u | | obj+0x3c | H |
| player height (=eye) | 48 u | | obj+0x34/+0x38 | H |
| crouch height | 48/4 = 12 u | | ext+0x24c=4 | H |
| crouch speed | 500/4 per 1.2 s (=3.1 u/frame @3 ticks) | | ext+0x254 | H |
| ceiling clearance | 8 u | | various | H |
| wall contact dist | r + 12 u (thick walls ≥ 40 u) | | `FUN_004a4a1f` | H |
| sub-step size | ≤ 8 u (single step if ≤ 12 u) | | | H |
| bounce | vz = −vz/2, <12 u → 0 | | `FUN_004a4307` | H |
| freefallhighdist | 128 u | | `DAT_0056710c` | H |
| hard-landing min | 96 u | | `DAT_00515530` | H |
| landing squat | FullHeight/6 = 8 u once | | OSETJUMP arg4 | M |
| kb turn | 24 /frame (fallback 20) | angle | `in+0x18`, 0x4a3179 | H |
| mouse yaw | ≤24: dx; else (dx/35+1)·24 | angle/frame | `FUN_0047a0c7` | H |
| mouse pitch | <24: dy; else (dy/30+1)·24 | pitch/frame | same | H |
| pitch range | ±512 | pitch units | `DAT_00567110/14` | H |
| kb look up/down | 24 /frame | | `in+0x20` | H |
| look left/right | 16 /frame, ±512 | angle | `FUN_0047ba30` | H |
| y-shear bias | 127 | | `DAT_00567118` | H |
| fly up/down/sink/pitch | 200/200/50/50 | u/1.2 s | ext defaults | H |
| fly sink threshold | 4 u above floor | | | H |
| maxflyheight | 0 (off) | absolute z | INI/OENGINE | H |

---

## 10. Struct offsets identified

### Object (480 bytes, same layout in XRB `Objects` and at runtime)
| off | type | meaning |
|---|---|---|
| +0x00 | byte flags | 0x01 immovable, 0x02 flying, 0x04 no friction, 0x08 dead/stop, 0x10 collides with walls, 0x20 collides with objects, 0x40 ?, 0x80 network player |
| +0x02 | byte | 0x04 local player-controlled |
| +0x06 | byte | 0x02 jumped, 0x08 confined to its region, 0x10 region-mask, 0x40 moving |
| +0x08 | short | object index (0 = player) |
| +0x0f | byte | 0x20 frozen |
| +0x14 | ptr | position point (x +0x14, y +0x18, z +0x1c) |
| +0x20..0x23 | u32 | action flags (section 3) |
| +0x24 | short | yaw (0..2047); `+0x22` is the same value as 16.16 |
| +0x28 | short | copy of view pitch |
| +0x2c | fx | H = feet z |
| +0x30 | fx | RH = z + Height (eye / head) |
| +0x34 | fx | Height (current) |
| +0x38 | fx | FullHeight |
| +0x3c | fx | radius |
| +0x40 | fx | weight (for pushing) |
| +0x7c/80/84/88 | fx | max speed x/y/z/turn |
| +0x8c/90/94/98 | fx | accel x/y/z/turn |
| +0xac/b0/b4/b8 | fx | speed fwd/strafe/vert/turn |
| +0xfc | ptr | current region |
| +0x124 | int | run-doubling active |
| +0x1ac | fx | type index |
| +0x1b0 | ptr | definition (effect args at +0x88.., sounds at +0x9c..+0xac) |
| +0x1b8 | ptr | player ext (section 5.1) |
| +0x1c8/+0x1ca | short | yaw / pitch amount this frame |

### Region (308 bytes)
| off | meaning |
|---|---|
| +0x0 | flags: 0x10 no floor (floor-link region at +0x5c), 0x20 no ceiling (link at +0x60), 0x80 check bbox width vs radius |
| +0x1 | 0x01 own physics block at +0x90 (fx,fy,g), 0x02 jump pad |
| +0x0c / +0x10 | floor / ceiling z |
| +0x6c / +0x70 / +0x74 | jump-pad speed / current bounce / mode |
| +0xb4 | object list head (next at obj+0x14c) |
| +0xb8 / +0xbc | wall pointer array / count |
| +0xc4,+0xc8,+0xcc,+0xd0 | bbox minx, maxx, miny, maxy |

### Wall
| off | meaning |
|---|---|
| +0x00 | 0x02 solid (never a portal), 0x04 has weight |
| +0x01 | 0x02 impassable portal |
| +0x02 | 0x01 skip, 0x02 thick |
| +0x0c / +0x10 | point pointers p1/p2 (x at +0x14, y at +0x18) |
| +0x80 / +0x84 | front/back regions |
| +0x8c | length |
| +0x90 | weight |

### Globals
| address | meaning |
|---|---|
| `DAT_00510778` | player object |
| `DAT_005675a0` | view |
| `DAT_005931dc` | input settings |
| `DAT_00510774` | movement model |
| `DAT_005676ac` | level Tail block (name at +8, physics at +0x20) |
| `DAT_005670f4` | step height |
| `DAT_00567100` | maxfly |
| `DAT_0056710c` | freefallhighdist |
| `DAT_005670e0..ec` | tick/time-scale values |

(Note: `(&DAT_0056c180)[type]` in the decompiler is a per-type table that only *appears* to start at
the playerspeedmaxt global. The real table base is probably 0x56c184, for index ≥ 1.)

---

## 11. Open questions
1. **Time unit in practice.** The code says `v·ticks/120` with `ticks = ms/10`, so 1.2 s per unit.
   Win9x GetTickCount granularity (~55 ms on 95/98?) would make ticks lumpy. Verify the real walking
   speed by measuring under Wine (debug HUD "PlrSpeeds"/"PlrPos").
2. **Frame-rate dependence.** Turning, look, mouse and the airborne fall accelerations are per
   frame. Decide whether the reimplementation reproduces them at a fixed ~29.4 Hz logic rate
   (recommended: run logic at the 33 ms cap with ticks=3) or rescales them.
3. Movement model: is M=0 really used everywhere? It is zero-initialised and no OPHYSON appears by
   name in the shipped XRBs, but effect objects might reference it by index. Check the XRB object
   type names for OPHYSON.
4. The exact floor-ramp (`e4`) formula near portals, and the corner-push details, still need careful
   re-derivation if exact feel matters.
5. OFLYON `Height = arg0` is copied without `<<16`. This is either a bug or arg0 is already 16.16.
6. Hard-landing damage: is event tag 0x3e6 (and 0x3b3 jump, 0x3ad crouch, 0x3ae run) handled by
   engine-side code (`FUN_00472b7c`)? Not traced.
7. Meaning of `ext+0x220` (default 2.0), `ext+0x25c`, and the non-player turn angle in Tail+0x2c.
8. Mouse axis-lock threshold `in+0x34`: default value not found (probably 0).
9. Handedness: confirmed consistent with the renderer (renderer.md §2): yaw 0 looks along +x, +y is to the left, angles increase counter-clockwise.
