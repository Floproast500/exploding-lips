# Mover ("bot") scripts – `[moveractioncodes]` interpreter (lips.exe)

Clean-room description of the G.O.D.D. mover language as used by Exploding Lips: how the
`[moveractioncodes]` section is compiled, how a script drives an object frame by frame, every
condition and opcode, and what the shipped bots actually do. Everything is restated from the
decompilation (`re/ghidra/decomp.c`, `re/ghidra/decomp_extra.c`); function addresses are
evidence pointers. Inventory of the shipped scripts: `python3 -I re/scripts/mover_scan.py
original [--scripts]`.

Confidence: **[H]** read from code, **[M]** code + data inference, **[L]** guess.
Related: `script.md` (§2 pre-processing, §3 INI command execution, §4 tags, §6 state values),
`combat.md` (projectiles, damage, OOTOOL/OOPROJ), `physics.md` (non-player physics),
`hud.md` (XM status line, OTEXT, "Talk:" line).

Units used below: **frame** = one engine frame (one call of the object's per-frame effect
update, at most ~30 per second because of the 33 ms frame cap, physics.md §2.4). Speeds are
16.16 world units per 1.2 s (`v·dt_ms/1200`). Angles 2048 per turn, 0 = +X, counter-clockwise.

---------------------------------------------------------------------------------------------

## 1. Data structures

### 1.1 Script records (compiled at orb load) **[H]**
`FUN_0044a718` reads `[moveractioncodes]` of the orb INI, `FUN_0044a4c6` walks it line by line
after the section pre-processing of script.md §2 (so `SetHitstokill` arrives as `SH`):

* A line that starts with `ONTAG` is executed as the INI command (stores a tag body).
* Otherwise the text before `=` (first 11 chars, upper-cased) is the script name and the text
  after `=` is copied twice (one copy is kept as the name-table entry, one is compiled in place).
  A line without `=` (the shipped `W(16)*` left over by XCHANGE7's missing `\`) still creates
  an empty, useless script named after the line. **[H]**
* At most 199 scripts per orb (`CompileMoverCodeBuffer TOO MANY` otherwise); the table is
  re-filled for each orb. Lookup by name (`FUN_00449cad`) is an exact, case-insensitive compare
  of the first 11 characters.

Script record, 180 bytes (table `DAT_0053329c`):

| off | meaning |
|---|---|
| +0x00 | name (char[12]) |
| +0x0C | pointer to the compiled text |
| +0x10 | flags: 0x10 has IN block; byte +0x11: 0x01 has ON blocks, 0x02 some ON uses `T(tag)` |
| +0x14 | IN body (NUL-terminated) |
| +0x18 | main body (text after the last ON block) |
| +0x1C/+0x20 | loop start and repeat count when the main body itself starts with `{` |
| +0x30 | OR of all ON condition masks |
| +0x34 + 32·k | ON block k (k = 0..3): +0 condition mask, +4/+8 body, +0x0C tag, +0x14 number argument, +0x18 string argument, +0x1C repeat count written after the closing brace |

### 1.2 Per-object mover state **[H]**
A mover lives in the object's **ObjectState** record (472 bytes, object +0x1B4; allocated on
demand by `FUN_004d7317`, which sets +0x9C = owner object). The same record holds the 16
"OS" state values at +0x198 (script.md §6.1), so a bot has its own Life/Magic/.../Power values.

| off | meaning |
|---|---|
| +0x04 (byte) | event bits: 0x01 script has ON blocks, 0x02 running an ON block, 0x08 *hit by player* pending, 0x10 *bumped by player* pending, 0x20 running the IN block, 0x40 re-scan ON blocks, 0x80 tag event pending |
| +0x05 (byte) | 0x01 has expiry time (+0x1D0), 0x02 `{…} ON cond` loop active, 0x04 *error* pending (`X` condition), 0x10/0x20 hit/bumped by another object, 0x40 "look for object in region" request, 0x80 bumped a wall |
| +0x06 (byte) | 0x01 following a path-map route, 0x02 (blocks activation), 0x04 *killed* (hit counter reached 0) |
| +0x12 | i16 angle step stored by `AI(n)`, used by `TA` |
| +0x14 | tag the object listens to for `T(tag)` conditions (16.16, integer part used) |
| +0x18 | virtual key for `WK` |
| +0x1C | text for `WU"…"` (≤ 20 chars) |
| +0x34/+0x4C/+0x64 | three extra names from OSETMOVE, usable as `%1`/`%2`/`%3` inside message strings |
| +0x7C..+0x9B | condition block of an active `{…} ON cond` loop; +0x84 = where to continue when it becomes true |
| +0x9C | owner object |
| +0xA8 | flags: 0x04 restart, 0x08 inside an `XC` call, 0x10 has IN, 0x20 IN already run, 0x40 de-activated by range, 0x80 activated by range |
| +0xAC / +0xB0 | original script record (for `XR`) / current script record |
| +0xB4 | start of the current sequence (target of `*`) |
| +0xB8 / +0xBC | program counter / start of the current instruction |
| +0xC0 | **wait counter** (frames) |
| +0xC4 | **action mask** handed to the physics every frame (bits = player action bits, physics.md §3.1) |
| +0xC8 | argument of `PM?`/`PT?` player actions |
| +0xCC / +0xD0 | loop start / remaining repeat count |
| +0xDC..+0xE8 | saved context of an `XC` call |
| +0xF4..+0x108 | saved main context while an ON block runs (start, pc, wait counter, action, loop) |
| +0x110 | copy of the script's OR-ed condition mask |
| +0x114 | index of the ON block being run (−1 none); +0x118: pointers to the 4 ON blocks |
| +0x128..+0x130 | path-map navigation state (`MM`) |
| +0x134 | the object's own XRB tag, saved when the mover is attached (object tag field is then reused for `WT`) |
| +0x154 | i16 **view cone** in angle units (from OSETMOVE; INITMOVES leaves 0) |
| +0x156 | i16 **other-tool index** selected by `P(n…)` (combat.md) |
| +0x198 + 4·i | state value i; **+0x1B0 = index 6 ("Power") is the hits-to-kill counter of `SH(n)`** |
| +0x1D0 | expiry time for spawned movers (object destroyed when passed) |

---------------------------------------------------------------------------------------------

## 2. Attaching scripts and activation

### 2.1 INITMOVES / OSETMOVE **[H]**
`INITMOVES=(A,B,…)` (`FUN_0040d7be`) builds, for each name, a zeroed OSETMOVE parameter block
whose *object* name and *script* name are both that name and runs the OSETMOVE effect
(`FUN_0043944d` → `FUN_00423bde` applies `FUN_004390fb` to **every object with that name**).
The explicit `OSETMOVE(object,script,confineregion,oncreatescript,flags,viewcone)` form
(`FUN_0040d6fd`, `%10s %10s %10s %10s %d %d`) is not used by the shipped INIs.

For each matching object (`FUN_004390fb`):
1. allocate its ObjectState if needed; save the object's XRB tag in state +0x134 and clear the
   tag (+0x46);
2. state +0xA8 = script flags, +0xAC = script; set object +0x06 bit 0x10 ("activated by area")
   and +0x13 bit 0x02 ("mover controlled");
3. home region = the confine region named in the block, else the object's region; its index is
   stored in object +0x4C/+0x4E and the region is flagged as a mover area (`+6 |= 0x30`);
   regions whose bounding box lies inside it get linked to it (`FUN_00450259`);
4. `FUN_0044a1bb` binds the script: pc = main body, wait = 0, action = 0, loop = script
   loop, ON block pointers, condition union; mark the object as mover (+0x11 |= 4) and put it
   on the MOVERS and ACTIVES lists; **if the script has an IN block and it has not run yet, run
   it now to completion** (`FUN_004493fa`, §3.4); if the script has a `T(tag)` condition, the
   object starts listening to that tag;
5. view cone = parameter 6 (0 with INITMOVES); then the activation test of §2.2.

Note: the shipped `INITMOVES=(…,XCHANGE8,XCHANGE)` asks for a script "XCHANGE" that does not
exist, so **XCHANGE9 is never attached** (no level places an XCHANGE9 object, so it is moot). **[H]**

### 2.2 Activation (who runs) **[H/M]**
Only movers flagged active (object +0x11 bit 0x08, on the ACTIVES list) execute.
* `FUN_004489f6(obj, player, 99)`: an "activated by area" mover is active iff the bounding box
  of the player's current region lies inside the bounding box of the mover's home region
  (`FUN_004d92a0`); a mover with object +0x06 bit 0x08 (`SV(CR=Y)`) is active only while the
  player is in the same region. Called at attach time and after `XN`.
* Entering a region (`FUN_004d85a1` region-change code) that is a mover area, or one linked to
  it, activates all movers in it (`FUN_004d81aa`); leaving deactivates (`FUN_004d8255`).
* Distance polling in the main loop (decomp ~66477): every 16th frame movers farther than
  **activatedistance** from the player and not in the player's region are switched off; 8
  frames later inactive movers closer than activatedistance are switched on. Default
  activatedistance = **1024** units (`DAT_0051022c`; `[startup] activatedistance` overrides;
  Lips does not set it). The distance is the 2-D distance stored per frame in the object's
  point +0x38.
* A bump/contact event (`FUN_00448e8b`) also activates a sleeping mover.
* `SV(AD=Y|N)` activates/deactivates explicitly (PLAYTRN1 uses `IN{SV(AD=Y)}` because it is
  not tied to a region); `SV(AE=N)` takes the object out of automatic activation.
* There is **no line-of-sight test** anywhere in activation or in `TP`.

### 2.3 Per-frame driver **[H]**
The bot objects use the **OTHING** effect (index 13; some levels use **OWALK**, index 5).
Per frame (event 0x10 of the effect handler, `FUN_00425361` OTHING / `FUN_00463537` OWALK):
1. decrement the contact debounce timer (object +0x118) by the frame's time step;
2. if the animation raised "fire now" (object +0x20 bit 0x02, set when an attack sequence hits
   an entry with next = 99) → fire the pending projectile (`FUN_00464e68`, combat.md);
3. if the state has an expiry time that has passed → destroy the object;
4. if the object is an active mover → `FUN_0044970d` (§3) returns the action mask:
   * bit 0x20000000 set → it is a *player action* (`PM?`, `PT?`, §4.6): apply to the player
     (`FUN_00425256`), give the bot no movement this frame;
   * otherwise object +0x20 (action flags) = mask, then normal object physics
     (`FUN_00467029` → `FUN_004a102a`, §5);
   * OWALK only: mask 0x4000000 (`MD`) means "do the built-in OWALK wandering instead".

---------------------------------------------------------------------------------------------

## 3. The interpreter

### 3.1 Compilation of one script (`FUN_0044a3f8`) **[H]**
```
script := [ "IN" "{" body "}" ]
          { "ON" cond "{" body "}" [ "(" n ")" ] }      -- at most 4 ON blocks
          main
main   := body      -- may itself start with "{" → loop
```
* `IN` must be followed by `{` (else "ERROR PreProcessINCodes"). The closing brace of IN and
  of every ON body is overwritten with NUL, so each body is a separate string.
* `cond` is a run of condition letters up to `{` (§3.5). More than 4 ON blocks: the 5th and
  later stay in the main text (and are then executed as junk).
* Everything after the last ON block is the main body.

### 3.2 Main loop per frame (`FUN_0044970d`) **[H]**
```
think(obj):
  st = obj.state
  if st == null or NOSEEUM toggled: return 0
  if st.has_on and st.rescan and (blk = scan_on_blocks(obj, st)):
      if not st.in_on_block: save main context (start, pc, wait, action, loop)
      st.start = st.pc = blk.body; st.wait = 0; st.action = 0; st.in_on_block = true
      return 0                                   # body starts executing next frame
  if st.until_loop_active and cond_true(st.until_cond):
      st.pc = st.until_continue; st.wait = 0; st.until_loop_active = false
  if st.wait >= 1: st.wait -= 1; return st.action    # keep doing the current action
  if obj waits for a tag (WT) / an animation end (WE) / a sound (WS) / a key (WK) / chat text (WU):
      handle that wait (§4.7); return 0 or st.action
  if st.action == MOVE_TO_POINT (0x800000) or MOVE_ALONG (0x400000): continue steering;
      while not arrived: return the steering mask; on arrival clear and fall through
  if not step(obj, st):            # end of script
      obj inactive, removed from ACTIVES
  return st.action
```
So events interrupt waits immediately, and **one opcode is executed per frame at most**
(plus any number of embedded INI commands, §3.3); an opcode that sets a wait of n frames keeps
its action for n further frames.

### 3.3 One step (`FUN_00448388`) **[H]**
```
step(obj, st):
  if st.wait > 0 and not st.rescan: st.wait -= 1; return 1
  pc = skip_blanks(st.pc); st.ipc = pc
  while (next = try_ini_command(pc)) != null: pc = next      # see below
  c = *pc
  if c == NUL:                          # end of a body
      if not end_of_sequence(obj, st, from_star=false): return 0
      pc = st.pc; c = *pc
  if c == '{':                          # loop head
      n, after = look_behind_matching_brace(pc+1)   # "}(n)" or "} ON cond"
      if n > 0: st.loop_count = n; st.loop_start = pc+1
      if "} ON cond": compile cond into st.until_cond, st.until_continue = text after cond,
                      st.until_loop_active = true
      pc = st.loop_start; c = *pc
  if c == '}':                          # loop tail
      if st.loop_count < 1: skip the "(n)" after the brace
      else: st.loop_count -= 1; pc = st.loop_start
      c = *pc
  if c == '*':                          # end of sequence → restart
      if st.until_loop_active: pc = st.loop_start
      else: pc = end_of_sequence(obj, st, from_star=true)
      c = *pc
  pc += 1; skip blanks
  dispatch on c:  U D F B R L H → move codes (§4.1); A (§4.2); G (§4.3); M (§4.4);
                  P (§4.6); S (§4.5); T (§4.2); W (§4.7); X (§4.8);
                  anything else → st.action = 0 (no-op, e.g. a stray ')')
  st.pc = pc after the opcode and its "(…)" argument group
  return 1
```
* **Embedded INI commands** (`FUN_00448313`): before each opcode, a word of **4 or more
  characters** (read with the INI delimiters) is looked up in the INI command table; if found
  it is executed exactly like an INI command line (script.md §3) and the scan continues after
  its argument group. This is how `OSOUND(…)`, `OTEXT(…)` and `XCHANGE(…)` work inside bot
  scripts. Words shorter than 4 characters are always mover opcodes, so the mover `XM`/`XT`
  shadow the INI commands of the same name. Quirk: the INI executor strips a trailing `)` or
  `}` from the whole remaining text once.
* `end_of_sequence` (`FUN_004480f3`):
  1. inside an `XC` call → return to the caller (restore start/pc/loop, wait = action = 0);
  2. inside an ON block → **resume the saved main context** (start, pc, remaining wait,
     action, loop), clear the hit/bump pending bits, no ON block running;
  3. restart flag (+0xA8 bit 0x04, set by OSETMOVE-by-object paths) → restart the script's main
     body;
  4. otherwise: reached via `*` → jump to the current sequence start (+0xB4: the main body, or
     the ON body when `*` is inside an ON block… which never happens in the shipped data);
     reached via NUL → **script ends** (object deactivated).
  So `*` = "loop the main sequence", and a main body without `*` runs once.
* Loop repeat counts: `{ body }(n)` executes the body **n+1 times** (the counter is decremented
  at each `}` while > 0). `{ body } ON cond` repeats until the condition becomes true (checked
  every frame before the wait counter). Not used by the shipped scripts.

### 3.4 IN block (`FUN_004493fa`) **[H]**
Run once at attach time, synchronously: the step function is called until it returns 0 (end
of the IN text). Waits inside IN are burned immediately. Then pc = main body, wait = action =
0. A second attach (`XG`, `XR`) does not run IN again for the same state.

### 3.5 ON conditions (compile `FUN_00444dc8`, actor letters `FUN_00444cef`, test `FUN_00448fcc`) **[H]**
Condition = event letter(s) + optional actor letter. Mask bits (u32):

| letter | bit | meaning when tested |
|---|---|---|
| `B` + actor | 0x200 | **bumped** (contact) by the actor |
| `H` + actor | 0x100 | **hit** by a projectile of the actor |
| `C` + actor | 0x400 | actor **close**: 2-D distance ≤ 2·(r_bot + r_actor) (player only) |
| `V` + actor | 0x800 | actor **visible in the view cone**: facing angle within ±cone/2 of the bearing to the player (no wall check; cone 0 with INITMOVES → never) |
| `T(tag)` | 0x8000 | tag event: the object listens to `tag` (it becomes the object's tag; tag marked "referenced" so ActivateTag scans objects) |
| `U"text"` | 0x10000 | last chat line typed by the player equals text (then the chat buffer is cleared) |
| `X`, `X~`, `XF` | 0x40000 | an opcode failed (named object missing, path-map node missing) in a script that asked for it |
| `E` / `L` | 0x1000 / 0x2000 | compiled but never tested |
| `F(n)` / `S(n)` / `M(n)` | 0x02 / 0x04 / 0x08 + number | compiled but never tested |

Actor letters: `P` player 0x10, `N` network player 0x20, `O` other object 0x40, `A` any 0x80,
`W` wall 0x80000, `C` 0x100000, `F` 0x200000, **`K` kill 0x400000**.

Test order inside one block (first match wins, `FUN_00448fcc`):
1. actor P: `H` and hit-pending and not bump-pending → **if the script has any `K`
   condition: hits counter (state value 6) −= 1, and if it is now < 1 set *killed***; clear
   hit-pending; true. `B` and bump-pending and not hit-pending → clear, true. `V` (cone) /
   `C` (close) when no hit/bump pending.
2. actor O: the same with the "other object" pending bits; the region-search request.
3. `BW`: true (clears the wall-bump bit).
4. `U"…"`: chat text match.
5. **`K` and hits counter ≤ 0 → true** (clears *killed*).
6. `T(tag)`: tag event pending and tag matches.
7. `X`: error pending.

`scan_on_blocks` (`FUN_00449323`, only while the object is active): clear the re-scan bit, test
blocks 0..3 in order, skipping the block that is currently running; the first true block runs.
If none matches, the tag-pending bit is cleared too.

**Events** that set re-scan (+0x04 bit 0x40):
* Object contact (effect event 0x200 → `FUN_00448e8b`), debounced per bot by object +0x118 =
  20.0 → one contact event per **240 ms** (20 / (100/120 per 10 ms tick)):
  * other object is the player and the script has a `…P` + `B…` condition → *bumped by player*;
  * other object is a projectile/explosion (object +0x02 bit 0x01 or +0x10 bit 0x80) whose
    owner (+0x144) is the player and the script has an `H…` condition → *hit by player*;
  * the contact also activates a sleeping mover.
* Wall touch (event 0x100 → `FUN_00448cf2`) for `BW`.
* Tag events (effect event 0x40 on an object listening for a `T` tag).
* Opcode failures (`X` condition).

Consequences for the shipped bots (`SH(n)`, blocks `HK`, `HP`, `BP` in that order):
* hits 1..n each run the **HP** block (the n-th hit drives the counter to 0);
* the **HK** block runs at the *next* event after that (the following hit or a bump),
  because HK is only evaluated during an ON scan. Whether one player shot produces two
  contact events (projectile + its explosion) decides if a bot really dies after n or n+1
  shots – confirmed in game: n+1 (§9 question 1). **[H]**
* `ON BP` fires whenever the player touches the bot (every 240 ms while touching).

---------------------------------------------------------------------------------------------

## 4. Opcodes

Argument syntax: numbers are decimal; `(a-b)` = uniform random in a..b where accepted
(`FUN_004445b5`); names may be bare words or quoted strings; strings support `"a"+"b"`
concatenation and `%1..%3` (OSETMOVE extra names) (`FUN_004442a8`, `FUN_004444a2`). A leading
`~` on `M`/`T` codes means "opposite" (face away). n = wait counter in frames.

### 4.1 Direct movement – letters `U D F B R L H`, combinable, e.g. `FR(10)` (`FUN_00447ef5`) **[H]**
| code | action bit (physics.md §3.1) | effect on a ground bot |
|---|---|---|
| `F(n)` | 0x10 FORWARD | accelerate forward for n frames |
| `B(n)` | 0x20 BACKWARD | accelerate backward |
| `L(n)` / `R(n)` | 0x40 / 0x80 | strafe left / right |
| `U(n)` | 0x100 UP | jump (vz = msz when on the floor) / fly up |
| `D(n)` | 0x200 DOWN | crouch (height shrinks) / fly down |
| `H(n)` | – | halt: action = 0, all four velocities zeroed |
The combined mask replaces the previous action; the animation action is chosen from it (§6).

### 4.2 Angle and turning – `A…`, `T…` (`FUN_00447780`, `FUN_004478b3`) **[H]**
| code | effect |
|---|---|
| `A(d)`, `AS(d)` | set facing to d degrees (d·512/90) |
| `AI(d)` | remember step = d degrees and turn by it |
| `AR(d)` | turn by ±d degrees, sign random |
| `TA` | turn by the remembered `AI` step (`~TA`: the other way) |
| **`TP`** | **face the player instantly** (angle = atan2 of the 2-D offset; `~TP` faces away). wait 0 |
| `TC` | face the centre of the current region's bounding box |
| `TO` | face an object found in the current region (`FUN_004d94b2`); fails → `X` |
| `TS(name)` | face the named object |
| `TL(n)` / `TR(n)` | set TURNLEFT / TURNRIGHT action for n frames (turn rate limited by mst, physics) |
| `TU(n)` / `TD(n)` | look up / down action (0x4000 / 0x8000) for n frames |

### 4.3 Teleport-style position changes – `G…` (`FUN_004454b9`) **[H]**
`GF(n)`/`GB(n)` x ± n, `GR(n)`/`GL(n)` y ± n, `GU(n)`/`GD(n)` z ± n (world axes, immediate),
`GC` jump to the region centre, `GO` back to the spawn point (`FUN_0042ccbc`), `GN(name)` /
`GS(name)` to a named object (`FUN_00443f0a`).

### 4.4 Goal-directed movement – `M…` (`FUN_00446064`) **[H for parsing, M for steering]**
| code | effect |
|---|---|
| **`MD(n)`** | action 0x4000000 for n frames: "do the object's own effect behaviour". OWALK bots wander with their built-in random walk (`FUN_00462d35`: forward, random turn bursts); OTHING bots just stand (walk animation, friction stops them) |
| `MF(P)` / `MF(name)` | follow the player / named object: target point refreshed every frame |
| `MS(name)` | move to the named object's current position |
| `MP` | move to where the player is now (snapshot copied into a scratch point) |
| `MC` | move to the region centre |
| `MO` | return to the spawn point (object +0x18) |
| `MA(name)` | move along/towards a named object (`FUN_0045a775`, action 0x400000) |
| `MM(name,n)` | path-map (OPMAP/OPMNODE) navigation |
Steering for MF/MS/MP/MC/MO (`FUN_0045a9d8`): every frame the facing snaps to the bearing of
the target; if the bot is already moving and the bearing changed by more than 64 units (11°)
it pauses forward motion for that frame, otherwise FORWARD; if the target is higher/lower the
bot switches to flying and adds UP/DOWN. Arrival (`FUN_00466b3b`) ends the move; arriving at
the spawn point snaps position and angle back to the spawn values. A missing target raises the
`X` condition if the script has one, else prints "<obj> <name> dosen't exist!".

### 4.5 Settings – `S…` (`FUN_00446c88`, `FUN_0044684c`) **[H]**
| code | effect |
|---|---|
| **`SH(n)`** | hits-to-kill: state value 6 of this object = n (used by the `K` logic, §3.5) |
| **`SV(list)`** | comma-separated settings: `AD=Y/N` activate/deactivate now; `AE=Y/N` mover-controlled flag (automatic activation); `CA=Y/N` activate by area; `CR=Y/N` active only in the player's region; **`ISi op v`** = player state value i (1..15) `= + - * /` v, raw (no clamping, no threshold tags) – `SV(IS10 + 1)` treasures found +1, `SV(IS12 + 1)` enemies killed +1 |
| `SS(a,b,c,d)` | set velocities forward/strafe/vertical/turn (−1 = keep) |
| `SI(a,b,c,d)` | add to those velocities |
| `SM(a,b,c,d)` | set max speeds msx/msy/msz/mst |
| `ST(k,d)` | start animation action k of the sprite's Tex; d = extra per-entry delay |
| `SZ(a,b)` | set object +0xE4/+0xF4, make it a flier |

### 4.6 Player/attack codes – `P…` (`FUN_004470f2`) **[H]**
| code | effect |
|---|---|
| **`P(n)` / `P(n,tag)`** | **attack**: select other-tool n (state +0x156 → OOTOOL[n]/OOPROJ[n]/OOXPLD[n], combat.md); start animation action 7; if the Tex has no separate action-7 sequence (all shipped bots) fire immediately (`FUN_00464e68`), else fire when the sequence reaches an end marker. The projectile's aux +0x90 = tag (default −1) – the tag fired when it hits the player (`P(1,411)` → ONTAG[411]: SUMOS(−5,Armor,Life) + taunt). Does not change the current action or wait |
| `PO(n)` | same as `P(n)` |
| **`PS(name,v)`** | play sound/OSOUND object *name* (`FUN_0040d5c6`, v passed to the sound loader **[L: volume/priority]**) |
| `PM?(a,b)` (`?` = F B L R U D S) | for b frames make the **player** do that action (only U → jump and D → crouch are implemented in `FUN_00425256`) |
| **`PT?(a,b)`** (`?` = L R U D) | for b frames turn the **player's** view: L/R yaw ± a units per frame, U/D pitch by a |
| `PJH`/`PJV(a,b)` | sets action 0x40000000/0x80000000 for b frames (no handler found) |
| `PXT(k)` | screen transition effect k = 1..5 (`FUN_0044381f`: 1 → (5,0), 2/3 → (3,0), 4 → (1000,100), 5 → (2000,50)) |

### 4.7 Waits – `W…` (`FUN_0044525f`) **[H]**
All W codes set the action to 0 (idle animation).
| code | effect |
|---|---|
| **`W(n)`** | idle for n frames |
| **`WT(tag)`** | make `tag` the object's tag and sleep until ActivateTag(tag) reaches it (OTHING event 0x40 clears the wait) |
| `WE` | wait until the current animation sequence reaches its end entry |
| `WK(c)` | wait until key c is pressed (`VkKeyScan`, `GetAsyncKeyState`) |
| `WS` | wait for a sound (sets a sound-system flag; **[L]**) |
| `WU"text"` | wait until the player types that chat line (meanwhile animation action 2) |

### 4.8 Misc – `X…` (`FUN_004457d3`) **[H]**
| code | effect |
|---|---|
| **`XT(n)`**, `XT(a-b)`, `XT*n` | ActivateTag(n) / random tag in a..b; `*` adds an object-specific offset (`FUN_00443e50`) |
| **`XM(col,"text")`** | status-line message in palette colour col (0 → 92), text up to 500 chars; commas inside the quotes are kept (unlike the INI `XM`) |
| `XS("text")` | message prefixed with the object's name in its own colour |
| **`XL(name)`** | spawn: clone of the object called *name* at the bot's position/angle (or a plain sprite whose Tex is *name* if no such object). `XL(DROPBONZ)` drops the DROPBONZ object: an OSOUND-effect object with Tex SKELPILE and sound BREAK01 (a pile of bones) |
| **`XN(name[,all])`** | **change class** (`FUN_00443c98`): copy the template object *name* (the hidden DEADBOTn / DEADKEYn objects) over this object, keeping position, region, z and links, and copy the template's whole ObjectState (script position, OS values) – the bot becomes the pickup and runs the template's script from where the template is. With a non-zero second argument every object with the bot's name is converted |
| **`XD`** | destroy self: action = DESTROY (0x10000000), deactivate, count in `DAT_00510218` |
| `XE` | send event 0x40 (tag event) to the object's own effect |
| `XF(n)` | set (n≠0) / clear the flying flag |
| `XC(name)` | call script *name* as a subroutine (one level) |
| `XG(name)` | switch to script *name* (re-bind, like OSETMOVE) |
| `XR` | restart the original script |

---------------------------------------------------------------------------------------------

## 5. How bots move (non-player physics `FUN_004a102a`) **[H]**
The action mask from the script drives the same accelerating model for every non-player
object (no instant max speed like the player):
* FORWARD/BACKWARD: vx ± Scale(ax) per frame, clamped to ±msx; LEFT/RIGHT on vy with ay/msy;
  TURNLEFT/RIGHT on vt with +0x98/mst.
* UP on the ground: vz = msz (a hop); while flying ±az up to msz.
* Friction every frame opposite to the velocity sign: fx, fy (level Tail +0x20/+0x24 =
  200/200), turn friction Tail +0x2C (568), halved in the air; gravity g (400) when walking.
  Friction is applied even while the key bit is held, so the effective forward acceleration
  is ax − fx.
* Facing is changed directly by `TP`/`A…`; turning by velocity only for `TL`/`TR`.

Shipped bot parameters (all DETHBOT/KEYBOT objects): msx = msy = 500, msz = 200, mst = 227,
ax = ay = az = 400, turn accel 2275, radius 16, height 48 (DEADBOT templates: msx 200, height
32). At the 30 fps cap (3 ticks/frame, Scale(v) = v·3/120): +10 −5 = **+5 units/1.2 s per
frame**, so an `F(16)` burst (17 frames + the attack frame) only reaches vx ≈ 90 and then
coasts to a stop in another ~18 frames: about **40 world units per script cycle** – the bots
creep. **[M: arithmetic from the code; not measured]**

`U(8)` on a hit: one hop, vz = 200 then gravity 400 → about 50 units high, ~1.2 s in the air.

---------------------------------------------------------------------------------------------

## 6. Animation actions (`FUN_004495e6`, flying variant `FUN_004494d4`) **[H]**
Whenever an opcode sets the action mask, the sprite's Tex action is chosen and, if it differs
from the current one (object +0x158), restarted at its start entry (Texs +0x24 u16[16]):

| mask | walking object | flying object |
|---|---|---|
| none (idle) | **2** | 8 |
| F/B/L/R or 0x400000/0x800000/0x4000000 | **3** walk (6 when running: object +0x20 bit 8 or mask bit 8) | 9 (12 running) |
| UP | 4 | 10 |
| DOWN | 5 | 11 |
| 0x200000 | 1 | 1 |
| bit 0x01 | 7 | 13 |
| attack (`P`) | **7** explicitly | – |
| waiting for chat text | 2 | – |
Action 0 = default/idle table entry. **All shipped bot Texs (LIPS01/02/04/06, BAGLADYA) have
an all-zero action table** (one looping sequence, e.g. LIPS06: 4 frames × 30 ticks, 1
rotation; BAGLADYA 16 rotations), so walk/attack/pain all show the same loop; the only visible
effect of an action change is that the loop restarts at entry 0. There is no pain or death
animation: pain = `U(8)` hop + message, death = `XL(DROPBONZ)` bone pile + `XN` turning the bot
into its pickup.

---------------------------------------------------------------------------------------------

## 7. Shipped scripts (all 36 INI copies; identical except DETHBOT9/DEADBOT9 in LIPS16)

Inventory (`mover_scan.py`): conditions `ON BP` ×1296, `ON HP` ×972, `ON HK` ×324 (all copies);
opcodes XT 1656, W 1296, XM 1296, SV 1008, F 648, P 648, TP 648, XN 648, SH 324, U 324, XD 324,
XL 324, MD 288, PS 36, PTL 36, WT 36; embedded INI commands OSOUND 468, OTEXT 324, XCHANGE 324;
structure IN 360, ON 2592, `*` 1620, stray `)` 1296 (harmless, see below).

| script | behaviour |
|---|---|
| **DETHBOT1..9** | `IN{SH(k)}` with k = 1,1,1,3,2,3,3,3,4. Main: `TP [OSOUND(GROWLnn)] F(16) P(t,41n) W(16)*` (bots 4,5,6,9 use `MD(16)` instead of `W(16)`; bot 9 uses other-tool 2, the others tool 1). One cycle ≈ 1 (TP) + 17 (F) + 1 (P) + 17 (W/MD) = **36 frames ≈ 1.2 s**: face the player, growl (bots 1–4), creep forward, shoot a projectile whose hit fires tag 41n (damage 5,5,10,15,25,30,50,75,90 to Armor then Life + taunt), pause. `ON HP {U(8) XT(761) XM(199,"OUCH…")}`: hop, random death-groan sound (tags 250–258), red message. `ON HK {XL(DROPBONZ) XT(761) SV(IS12 + 1) XN("DEADBOTn")}`: bone pile, groan, enemies-killed +1, turn into the pickup (boss 9 also OTEXT "YOU KILLED THE LEVEL BOSS!" + XT(409) = Life +100, Experience +1000; in LIPS16 the text is on DEADBOT9's pickup instead). `ON BP {XM(199,"GOTCHA!!!") XT(661)}`: **touching a death bot kills the player** (tag 661 = death: deaths +1, Life/Armor restored to 100/50, gore sound, PLAYTRN1 spin + teleport) |
| **DEADBOT1..9** | pickup templates: `ON BP {XT(40n) OTEXT("YOU FOUND …") SV(IS10 + 1) XD} W(1)*` – touching it gives the item (ONTAG 401–409), shows the text, treasures +1, disappears |
| **KEYBOT1..9** | no SH/K: `ON HP {SV(IS12 + 1) XN("DEADKEYn")}` – first hit converts it into the key pickup DEADKEYn (an OPICKUP object, keys KEY02..KEY09); `ON BP`: KEYBOT1 kills (661), the others `XT(3)` (DEATH01 OTELE: teleport back to the start without the death penalty). Main `TP F(16) P(2) W(16)/MD(16) *` – shoots other-tool 2 with no tag |
| **DEADKEY1..9** | `W(1)*` idle; the pickup itself is handled by the OPICKUP effect |
| **XCHANGE1..9** | traders: `ON HP {OSOUND(BLUB01) XM(195,"offer")}` (shoot to hear the offer), `ON BP {XCHANGE(give,failTag,get,okTag)}` (touch to trade, tags 501–518), main `W(16)*`. XCHANGE7 lost its main body (missing `\`): its script ends at once and the object sleeps, but a contact re-activates it, the ON block runs and the trade still works. XCHANGE8 gives nothing (`Experience` reduced to `E`, script.md §6.1). XCHANGE9 is never attached (§2.1) |
| **PLAYTRN1** | `IN{SV(AD=Y)}` always active. `WT(661) PS(MWEP7,1) PTL(200,10) XT(3) *`: on every player death play MWEP7, spin the player's view 200 units/frame for 11 frames (≈ 1.07 turns), fire tag 3 (DEATH01 teleport to start), wait for the next death |

Stray `)`: e.g. `XN("DEADBOT1"))` – after `XN` the program counter comes from the template,
so it is never reached; in `ON BP {… XT(661) )}` the `)` is executed as an unknown opcode (one
extra frame with action 0) before the block ends. **[H]**

---------------------------------------------------------------------------------------------

## 8. Reimplementation summary

```
struct Mover { script, start, pc, wait, action, loop_start, loop_count,
               in_on: Option<SavedCtx>, on_index, ev_hit, ev_bump, ev_tag, rescan,
               hits_left /*state[6]*/, view_cone, tool_index, listening_tag }

per frame, for every active mover object (OTHING/OWALK update):
    if fire_pending_from_animation: fire_other_tool(obj)
    mask = think(obj)                     // §3.2: ON scan, wait countdown, one step
    if mask & PLAYER_ACTION: apply_to_player(mask, param); continue
    obj.actions = mask                    // OWALK: mask == MD → own wander AI
    physics_non_player(obj)               // §5

on contact(bot, other) with 240 ms debounce:
    activate(bot)
    if other == player and script_has(B,P): ev_bump = rescan = true
    if other.is_projectile_or_explosion and other.owner == player and script_has(H): ev_hit = rescan = true
on tag t reaching an object waiting in WT: clear the wait
```
Keep: one opcode per frame; embedded ≥4-char INI commands; `*` restarts the main body; ON
blocks preempt and resume the saved main context (with its remaining wait); HK fires on the
scan after the hit counter reaches 0; facing changes are instant; no line of sight.

---------------------------------------------------------------------------------------------

## 9. Open questions
1. ~~Does one player shot give a bot two contact events?~~ **Confirmed in game (2026-10-12)**
   with `tests/pilot/shots_to_kill.txt` (godd-launch teleports the player and reads the mover
   hit counter and the kill count): the shotgun's first shot takes DETHBOT1's SH(1) counter to 0
   with no kill; the next *fired* shot runs HK. The 1-frame BLACK explosion adds no second
   event, and tags the bot does not listen for (its own `XT(761)`) do not trigger an ON scan.
   So `SH(n)` bots die on shot n+1 with instant weapons. Caveat for testing: tools with
   `NUMS[3] >= 1` ignore presses until the use animation is idle again (shotgun 120 cu ≈ 1.44 s).
2. Exact meaning of the `PS` second argument and of `WS`; `PJ?` actions have no consumer.
3. The view-cone test does not handle wrap-around at angle 0/2048 (harmless: unused).
4. `FUN_004d94b2` (object search for `TO` and the O-actor) was not traced.
5. Activation geometry: the bounding-box containment rule (`FUN_004d92a0`) was read from code,
   not checked against level data; verify which DETHBOTs wake when entering which rooms.
6. Frame-rate dependence: waits are counted in frames, so bots act faster on a machine that
   reaches the 30 fps cap than on a slow one. A reimplementation should run mover logic at a
   fixed 30 Hz.
