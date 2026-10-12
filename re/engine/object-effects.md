# Object effect handlers (lips.exe, G.O.D.D. engine)

This is a clean-room description of the object ("O…") effects that the shipped Exploding Lips levels
use. It covers four things: how effects are dispatched, the event codes, contact ("bump") detection,
and per-effect behaviour.

Sources:
- The decompilation: `re/ghidra/decomp.c` and `decomp_extra.c`.
- Disassembly with objdump, used where Ghidra lost register arguments.
- Cross-checks against all 19 shipped XRBs with `re/scripts/effect_dump.py`:
  - `--counts` gives usage per effect;
  - `EFFECT…` dumps objects with their aux record;
  - `--raw` adds a word dump.

Related docs: `effects.md` (effect table, whose order gives the effect index), `../formats/XRB-records.md`
(record layouts) and `script.md` (tags, INI commands, OS thresholds, menus/GAME* in §8).

Confidence: **H** = read directly from the code path, **M** = code plus data inference, **L** = guess.

Notation used in the effect sections:

| term | meaning |
|---|---|
| `self` | the object owning the effect |
| `other` | the extra object passed with the event (EBX) |
| `aux` | `self+0x1b0` |
| `s0..s9` | aux name slots at `aux+0x0e+12k`. s0–s4 are sound names; their resolved Sounds are `S0..S4` at `aux+0x9c+4k` |
| `int0..int4` | `aux+0x88..+0x98` |
| `k54` | object +0x54 |
| "edge test" | return if `self.+4 & 0xf` (already in contact, see §1.5) |
| "player test" | `FUN_0048a05a(other) != 0`: the player, or a projectile owned by the player |

## 0. Summary

Usage counts are numbers of objects in the shipped levels. Event keys: 1 init, 4 sound-stopped, 8
post-tick, 0x10 tick, 0x40 tag, 0x80 trigger, 0x100 hit-wall, 0x200 bump. All are explained in §1.2.

| effect | idx | handler | uses | events handled | what it does | conf |
|---|---|---|---|---|---|---|
| OTHING | 13 | 0x425361 | 707 | 1, 0x10, 0x40, 0x80, 0x100, 0x200 | pushable thing driven only by its mover script (INITMOVES). The tick steps the script; bump/wall/tag feed script ON-conditions; bump has a 0.24 s cooldown | H/M |
| OWALK | 5 | 0x463537 | 307 | 1, 0x10, 0x40, 0x100, 0x200 | walking bot. Mover script if attached, else path or built-in random walk. Writes the action word +0x20 and lets physics move it. Bump/hit: FUN_0045fe91 reward test, hit counter k54 fires its own tag | M |
| OTEXT | 132 | 0x43e612 | 302 | 1, 0x10, 0x40, 0x80, 0x200, 0x80000 | show a text box for int3 seconds (default 5), at most int4%1000 times. Text is aux s5..s9 joined (kind k54 = 0); other kinds read a file, INI section or namedmessage | H |
| OPICKUP | 130 | 0x45dc0e | 251 | 1, 0x10, 0x40, 0x80, 0x200 | on player bump: put the item into the player's 64-slot inventory (controller +0x264, count +0x364), hide it, fire tag int0, play S4. No stacking, no respawn | H |
| OSOUND | 53 | 0x42e442 | 231 | 1, 4, 0x10, 0x40, 0x80, 0x200, 0x100000 | play S0: int0 = count/loop, int1 = volume, int2 = positional/restart digits, int3 = falloff, int4 = delay (1/10 s) | H/M |
| OBUMPSW | 22 | 0x4295bc | 162 | 1, 0x80, 0x200 | bump switch, player body only (or filter int4): trigger s9 and fire its own tag; flag 0x1000 limits uses to k54 | H |
| OTAGSW | 20 | 0x429090 | 93 | 1, 0x80, 0x200 | as OBUMPSW, but player projectiles also count (shootable, +0x1d3 bit 0) | H |
| OHOTSPOT | 84 | 0x441adf | 88 | 1 | menu item; selection and activation are done by OMENU. Enter fires tag int3 | H |
| OSETF / OSETC | 41 / 49 | 0x43c6de / 0x43cb39 | 68 / 4 | 1, 0x10, 0x40, 0x80, 0x200 | move floor/ceiling of region s9 to height int0 at speed int1. Optional texture s8; sounds S0/S1/S2 for start/loop/stop | H |
| OTIMER | 80 | 0x42c5ff | 46 | 1, 0x10, 0x40, 0x80, 0x200, 0x100000 | one-shot delay of k54 tenths of a second, then trigger s9 and play S4. Restarting pushes the deadline back; OEFFOFF cancels | H |
| OTELE | 15 | 0x42d2ff | 45 | 1, 0x40, 0x80, 0x200 | teleport the player (or the bumping player-type object) to object s9 (copying its look/stats too) or to region s9. k54 ≠ 0 picks a random region | H |
| OWTOW | 67 | 0x426cba | 45 | 1, 0x40, 0x80, 0x200 | copy the textures/flags/effect of wall X onto wall s9 (doors, gates, lights) | H |
| ODROP | 131 | 0x45e4b2 | 43 | 1, 0x40, 0x80, 0x200 | take item s9 out of the player's inventory, place it at point s8, fire tag int0 | H |
| OSPOT | 61 | 0x4299b2 | 34 | 1 | hidden, inactive named marker (teleport destinations / templates) | H |
| OOTOO | 66 | 0x427c70 | 21 | 1, 0x40, 0x80, 0x200, 0x4000000 | morph object(s) named s9 into template s8 (switch flip, reveal, player morph) | H |
| OSNDSEQ | 54 | 0x42f615 | 21 | 1, 4, 0x10, 0x40, 0x80, 0x200 | chain S0..S4, each played max(1,int k) times; k54 = repeats (−1 endless); retrigger toggles it off | H |
| OCURSOR | 83 | 0x441430 | 19 | 1 | menu cursor overlay (picture s9), driven by OMENU | H |
| OFLYON / OFLYOFF | 26 / 27 | 0x429d26 / 0x42a0da | 18 / 18 | 1, 0x40, 0x80, 0x200 | player flying on (flags \|= 2, fly speeds int1..4, height int0) / off | H |
| OSTRAP | 29 | 0x42ae89 | 17 | 1, 0x40, 0x80, 0x200 | level exit: load level s9, arrival object s8, keep inventory and stats | M/H |
| OSPAWN | 62 | 0x4299f4 | 17 | 1 | same as OSPOT; no consumer found in the shipped game | H/L |
| OORIENT | 69 | 0x46423b | 10 | 1, 8 | copy angle+int0 and z from leader s9 every frame; destroyed with its leader | H |
| ORANDOM | 81 | 0x42c805 | 7 | 1, 0x10, 0x40, 0x80, 0x200 | random delay in [int2, int3] tenths of a second, then trigger s9. Mode int0: 0 = repeat forever, 1 = one shot per start | H |
| OVIEWON / OVIEWOFF | 181 / 182 | 0x437702 / 0x437820 | 5 / 2 | 1, 0x40, 0x80, 0x200 | enable/disable view s9 (TV screen); OFF can clear it to colour int0 | H |
| OPHYSON | 172 | 0x438061 | 4 | 1, 0x40, 0x80, 0x200 | set the global movement model to k54 | H |
| OINCOS | 112 | 0x44be5d | 4 | 1, 0x40, 0x80, 0x200 | add int0..int4 to Life/Magic/Armor/Money/Exp of s9 or the player, through the thresholds | H |
| ODESTROY | 78 | 0x42684a | 3 | 1, 0x40, 0x80, 0x200 | destroy object s9 next frame, play S4 | H |
| OFOLLOW | 10 | 0x46038f | 3 | 1, 0x10, 0x40, 0x100, 0x200 | follow object s9 or a path via OPATH holder; can carry s6 and shoot | M |
| OPATH | 171 | 0x429948 | 3 | 1 | path holder: +0x110 = Path s9 | H |
| OFLASH | 141 | 0x4307c3 | 2 | 1, 0x10, 0x40, 0x80, 0x200 | lightning: toggle fullbright k54+1 times (on int0, gap int1, in 1/10 s; negative = random), play S4 | H |
| OSELTOOL | 127 | 0x464ae9 | 2 | 1, 0x40, 0x80, 0x200 | select tool k54 for the player (int1 can make it available first) | H |
| OEFFSW | 73 | 0x42b7d0 | 2 | 1, 0x40, 0x80, 0x200 | relay: trigger s9 and play S4; optional use limit | H |
| OCOUNT | 72 | 0x42b5cb | 1 | 1, 0x40, 0x80, 0x200 | count k54 events, then trigger s9, play S4 and fire tag int3; reloads | H |
| OTEXOVER | 86 | 0x436981 | 1 | 1, 0x10, 0x40, 0x80, 0x200, 0x80000, 0x100000, 0x200000 | toggle a full-screen picture overlay (s9, e.g. rain). Auto mode shows it in regions without a ceiling | H |

**Things that differ from earlier assumptions:**
- OSETF and OSETC are "set floor" and "set ceiling". They are not OS-state effects.
- OPICKUP int0 is a **tag**, not a value.
- OWTOW and OOTOO are appearance copies, not teleports.
- OTIMER, ORANDOM and OFLASH run on a real-time clock in tenths of a second.
- ActivateTag passes an activator (EDX), which is the `other` of event 0x40.


## 1. Effect dispatch, events and contact ("bump")

### 1.1 Effect table and handler signature **[H]**

- `FUN_004727f4(name, handler, mask)` registers an effect. The registrations are all in
  `FUN_00467054` and run in the order of `effects.md`. That order is the **effect index** stored in
  XRB records: object +0x140, wall +0xd0, region +0xb0 and view +0x68. After load the fix-up turns
  the index into a pointer to the table entry.
- Table entry layout: +0x00 u32 **event mask**, +0x04 name, +0x10 handler.
- Handler signature (Watcom register call). In Ghidra output: `param_1` = self, `unaff_EDX` =
  event, `unaff_EBX` = other.

  ```
  handler(self: *Record /*EAX*/, event: u32 /*EDX*/, other: *Object or 0 /*EBX*/, entry: *EffectEntry /*ECX*/)
  ```

- `self` is the object, wall, region or view that owns the effect. `other` depends on the event
  (see §1.3): the bumping object, the switch, and so on.
- **Guarded dispatch** `FUN_00472a54(entry, self, event, other)` calls the handler only if all of
  these hold:
  1. `entry != NULL`
  2. `(entry.mask & event) != 0`
  3. `self.+4 & 0x80000000 == 0` (effect switched off by OEFFOFF)
  4. not (`self.flags & 0x01000000` and `self.+4 & 1`)

  Most senders use this. A few call the handler **directly without the mask test**: events 1, 2,
  0x2000000, and the active-list ticks 0x10 and 0x08 (which use their own mask test).

### 1.2 Event codes **[H unless noted]**

| event | name (ours) | sender / when | `other` (EBX) |
|---|---|---|---|
| 0x00000001 | INIT | `FUN_00472f90(1)` at level start: every object, wall, region and view, no mask test. Also sent alone to spawned objects (`FUN_0048c4bb`) | 0 |
| 0x00000002 | SHUTDOWN | `FUN_00472f90(2)` at level unload / exit (`FUN_004725a3`, `FUN_004d64b2`) | 0 |
| 0x00000004 | SOUND-STOPPED | sent directly (no mask test) by the sound layer when a Sound record that points back to an owning object is stopped by name (`FUN_0049d489`/`FUN_0049d411`); handled by OSOUND/OSNDSEQ | – |
| 0x00000008 | POST-TICK | per frame, to active-list entries whose mask has 8, after movement (`FUN_004a0375`) | 0 |
| 0x00000010 | TICK | per frame, to active-list entries whose mask has 0x10, before movement | 0 |
| 0x00000020 | view | views only (`FUN_004ab15f`) | – |
| 0x00000040 | TAG | ActivateTag(t, activator) (`FUN_00472b7c`, EAX = tag, EDX = activator): every object/wall/region with int(tag) == t. Broadcast happens only for t >= 999, t == 9999, or tags flagged in `DAT_0056bd8c`; `FUN_00472d69` sets that flag at load for every tag 0..999 that an object/wall/region with mask&0x40 listens to, so in practice every listened tag reaches its listeners | the activator passed to ActivateTag (switches pass the bumper; several callers leave EDX stale, so treat as unreliable) |
| 0x00000080 | TRIGGER | switch-type effects (OTAGSW, OBUMPSW, OOTAGSW, ORANTAGS, OCOUNT, OTIMER, ORANDOM, OEFFSW, ...) send it to the object named in their aux slot 9 (aux+0x7a), found with `FUN_00424668` | the sender (usually) |
| 0x00000100 | HIT-WALL | moving object hit a solid wall (`FUN_004a4a1f`), to the object | – |
| 0x00000200 | BUMP | object–object contact (both objects get it), or object crossed/touched a wall with flags&1 and tag ≥ 0 (to the wall) | the other object |
| 0x00000400 | FLOOR | object landed on floor (`FUN_004a41e3`), to object and region | – |
| 0x00000800 | CEILING | object hit ceiling (`FUN_004a4434`), to object and region | – |
| 0x00001000 | ENTER-REGION | object (player, or any object with flags&0x80) changed region: to new region and object (`FUN_004d7fc6`, `FUN_004d8089`) | – |
| 0x00002000 | LEAVE-REGION | same, to old region and object | – |
| 0x00010000 | PLAYER-MOVED | to the player object when its position changed (`FUN_00493d69`), and from `FUN_00456e36` | – |
| 0x00080000 | UI-KEY | to the current UI object `DAT_0051020c` (menus/text input; `FUN_00477f40`) | – |
| 0x00100000 | EFFECT-OFF | OEFFOFF (`FUN_00433547`) to its target, which also sets target.+4 \|= 0x80000000 | – |
| 0x00200000 | EFFECT-ON | OEFFON (`FUN_00433215`) to its target, after clearing the disable bit | – |
| 0x00400000 | REGION-WAKE | `FUN_004d81aa`: the player enters a region whose flags2 has 0x100000 (or its group via region +0x106), or a sleeping mover comes within `activatedistance`. Sent to objects in that region whose mask has 0x400000 | – |
| 0x00800000 | REGION-SLEEP | `FUN_004d8255`: the player leaves such a region. Also sent from object destruction to objects in its region | – |
| 0x01000000 | NOTIFY-DESTROYED | `FUN_004a3839` (destroy) to an object found by the destroyed object's name | – |
| 0x02000000 | CREATE | at XRB load (`FUN_0047cb09`), directly to every object whose mask has 0x2000000, before INIT | 0 |
| 0x04000000 | ? | only OOTOO declares it (see that section) | |
| 0x10000000 | (mask flag of GAME*) | | |

How to read the mask column in `effects.md`: the mask is just the OR of the events the handler
wants. For example OTHING 0x3d0 = BUMP | HIT-WALL | TRIGGER | TAG | TICK.

### 1.3 Active list and per-frame order **[H]**

**Active list.** The active list (`DAT_00567598`, up to 4096 `(entry, target)` pairs, count
`DAT_00567524`) is filled at load by `FUN_0047291d(entry, target)`. A pair is added only if
`entry.mask & (8|0x10)`. `FUN_004729ae` removes a target from it, for example on destroy.

**Frame order** (`FUN_004a0375`):
1. Compute the frame step `dt` (below).
2. **TICK 0x10** to each active pair whose mask has 0x10. Objects with flags&0x01000000 (all
   shipped objects) are skipped while any of these holds:
   - their region has flag 0x800000 and is not awake (region.+4 & 3 == 0);
   - the object has +4 bit 0x2000 (inactive);
   - their region has flag 0x400000 and the player is not in it.

   Walls (flags 0x02000000) are skipped when both their sides are the region `DAT_005106f0`.
3. For every object:
   - If +0x20 has 0x10000000 (pending destroy), call destroy `FUN_004a3839`.
   - Otherwise, if it is awake (+4 bit 0x10000000, set by `FUN_00467029`, or +0x1d3 bit 0), is in
     a region and is not inactive: clear the wake bit and run **UpdateObject** `FUN_004a4a1f`
     (movement, wall/object collisions, which send events 0x100/0x200/0x400/0x800/0x1000/0x2000).
   - Otherwise, step its sprite animation unless it is hidden.
4. **Contact latch** (objects in a region). For each pair (even bit = "this frame", odd bit =
   "last frame") in +4: (0,1), (2,3), (4,5), (6,7) and (+5 bit0, +5 bit1):
   - odd := even;
   - then clear all even bits (`&= 0xfeaa`).

   Objects without a region get +4 low 10 bits cleared.
5. **POST-TICK 0x08** to active pairs whose mask has 8.
6. Region effects and the rest of the frame.

**Frame step** (`FUN_00473107`, global `DAT_005670ec`, 16.16):

```
n  = clamp((GetTickCount_now - last) / 10, 1, 20)   // 10 ms units
dt = n * 100 / 120                                  // DAT_005670dc = 120.0 (FUN_00472605)
```

- So **1 time unit = 12 ms, and one second ≈ 83.3 units**. At most 16.7 units pass per frame.
- Per-frame cooldowns count down by `dt` (e.g. OTHING +0x118 = 0x140000 = 20 units ≈ 0.24 s).
- **Wall-clock timers are different**: OTIMER, ORANDOM, OFLASH use `FUN_0049fc30` = real time in
  **tenths of a second** (GetTickCount based, `FUN_0049fa1c`); `FUN_0049fbe4` = time in seconds
  (used for state-block lifetimes).

### 1.4 Object flag words used by effects

`flags` = object +0x00 (from XRB, mostly static). `rt` = object +0x04 (runtime; zero in the
file except as noted).

| where | bit | meaning | conf |
|---|---|---|---|
| flags | 0x1 | pass-through: no push-apart in object collisions (other events still sent) | H |
| flags | 0x2 | bounce: vertical speed reflects on floor contact. Set on some decorations and pickups | M |
| flags | 0x10, 0x20 | object is moved/collided by UpdateObject. 0x20 = takes part in object–object collision on either side | H |
| flags | 0x80 | mover/bot ("player-ish" for activator tests; region enter/leave events) | M |
| flags | 0x200 | immovable: when bumped, the mover is put back instead of pushing this object. All switch/trigger effects set it on INIT | H |
| flags | 0x400 | ignored by object collision | H |
| flags | 0x1000 | (switches) limited number of uses, count in +0x54. All OWALK movers have it too, meaning there unknown | H/M |
| flags | 0x10000 | projectile-like: contact sets rt bit0 instead of bit2 on the other object | M |
| flags | 0x40000 | the player | H |
| flags | 0x200000 | projectile with owner +0x144 | M |
| flags | 0x01000000 | normal world object; all shipped objects. Enables region sleep/wake gating of ticks | M |
| rt | 0x01/0x02 | contact with projectile-like object this/last frame | H |
| rt | 0x04/0x08 | contact (bump) this/last frame | H |
| rt | 0x10/0x20 | hit wall this/last frame | H |
| rt | 0x40/0x80 | floor contact this/last frame | H |
| rt | 0x100/0x200 | ceiling contact this/last frame | H |
| rt | 0x1000 | **hidden** (not drawn, not animated). OHIDEOBJ sets it; OSPOT/OSPAWN records have it in the file | H |
| rt | 0x2000 | **inactive** (no tick, no collision). OSPOT/OSPAWN have it in the file | H |
| rt | 0x10000000 | wake: run UpdateObject next frame (`FUN_00467029`) | H |
| rt | 0x20000000 | destroyed (`FUN_004a3839`; that function also sets tag +0x46 = -1 and unlinks the object) | H |
| rt | 0x80000000 | effect disabled (OEFFOFF). Dispatch drops all guarded events | H |
| +0x20 | 0x10000000 | pending destroy at next frame | H |

### 1.5 Bump (touch) detection **[H for the code path, M for exact flag semantics]**

Inside UpdateObject, a moving object A (flags & 0x20) tests every other object B in each region it
overlaps (region lists +0xb4/+0x14c). B is a candidate if all of these hold:
- B.flags & 0x20;
- not B.flags & 0x400;
- not B.rt & 0x2000;
- they are not the same owner group (+0x144);
- they are not owner and projectile of each other;
- the extra +0x0c/+0x10 exclusions hold;
- their z ranges overlap: `B.z ≤ A.z + A.h` and `A.z ≤ B.z + B.h`;
- `|Δx| < rA + rB` and `|Δy| < rA + rB` (a square test, radii at +0x3c).

On contact:
1. Unless either object has flags&1, the objects are pushed apart. If B has flags&0x200, A is put
   back to its old position instead.
2. **BUMP 0x200 is sent to A (other = B) and then to B (other = A)** when one of these holds:
   - exactly one of them is "player-ish" or player-owned (`FUN_0048a05a`);
   - both are player-ish (`FUN_00489ff8`);
   - certain projectile/owner combinations apply;
   - A has +0x1d3 bit 0.
3. After each send, the receiver's rt gets bit 0x04, or 0x01 if the other is projectile-like.

So static objects are bumped only by moving objects: the player, bots and projectiles. Because
the rt bits are set *after* the handler runs and are latched each frame, a handler that tests
`rt & (1|2|4|8) == 0` reacts only on the **first frame of a contact**. Standing on or against a
trigger therefore fires it once. Stepping away for a frame and touching it again re-arms it.

Walls: when the mover crosses or touches a wall with wall.flags & 1 and wall tag +0xa6 ≥ 0, the
wall's effect gets 0x200. This happens once per wall in a row (`DAT_00515538`). A solid wall hit
also gives the mover 0x100.

There is **no "use" key**. Interaction happens by touching (bump), by firing a tool (projectiles
bump as player-owned objects) and by tags (see `original/keyboard.txt`: tools are "selected by
touching special objects").

### 1.6 Common trigger idiom of the O* command effects **[H]**

Most "command" effects (OHIDEOBJ, OSHOWOBJ, OEFFON/OFF, ODESTROY, OSETF, OSOUND, OFLYON, OVIEWON, …) share one
template:

```
on INIT (1):   flags |= 0x200 (immovable); preload the aux sounds (FUN_0049e0bd)
on TAG (0x40) or TRIGGER (0x80): fire()
on BUMP (0x200): if FUN_0048a05a(other) != 0 (player or player's projectile)
                 and rt & (1|2|4|8) == 0 (first contact): fire()
fire(): act on the target named in aux slot 9 (aux+0x7a), parameters in aux ints +0x88..
```

The individual sections below list only how each effect differs from this template.

Helpers used throughout:

| function | meaning |
|---|---|
| `FUN_00424668(name)` | find a named entity: object (`FUN_00424862`), then wall, region, view, point, path |
| `FUN_00424862(name)` | find an object by name |
| `FUN_0042426a(self, other, code, &out)` | activator filter by decimal digits of `code` (see OBUMPSW/OTAGSW) |
| `FUN_0048a05a(o)` | o if it is the player; the owner if o is a player-owned projectile; else 0 |
| `FUN_00489ff8(o)` | o is player-ish (flags 0x40000, flags 0x80 or +0x10 0x200000) |
| `FUN_00472b7c(t)` | ActivateTag |
| `FUN_00467029(o)` | wake o for UpdateObject |
| `FUN_004a3839(o)` | destroy o |
| `FUN_0049e0bd(aux)` | resolve/preload the five aux sound names (slots 0–4) into the Sound pointers +0x9c.. |


---

## 2. Per-effect reference

The sections are grouped by subsystem. Each group ends with a table of the helper functions decoded while working on it.

<!-- part A -->

Notation: `self` = target object (EAX), `ev` = event (EDX), `other` = EBX. `obj+0x10` is the 2nd flags dword
("flags2", all 0 in shipped data for OTHING; set at runtime). `S` = object state block (object +0x1b4, the
mover/bot runtime block allocated when a `[moveractioncodes]` script is attached, 472 B). "Edge test" = return if
self contact bits (+4 & 0x0f) != 0. Time unit: DAT_005670ec per frame (coordinator: 1 unit ~ 12 ms).

---------------------------------------------------------------------------------------------------------------

### OTHING (index 13, FUN_00425361, mask 0x3d0) - confidence H (handler), M (mover-block semantics)

The generic "thing": a physical object whose behaviour comes entirely from an attached mover script
(`INITMOVES`/`OSETMOVE`). Without a script it is just a pushable solid that wakes the physics each frame and
ignores tags/triggers. All 707 shipped OTHINGs are bots/pickups driven by mover scripts (DETHBOT*, DEADBOT*,
XCHANGE*, KEYBOT*, DEADKEY*, PLAYTRN*).

**Guard:** if self is destroyed (+4 bit 0x20000000) do nothing.

**Trigger events**

| ev | action |
|---|---|
| 0x01 init | load aux sounds (FUN_0049e0bd) if aux; cooldown +0x118 = 0; if no home point (+0x18) allocate one as a copy of the current point (+0x14) with point+0x34 = angle<<16. (Does NOT set flag 0x200: things stay pushable.) |
| 0x10 tick | see pseudocode: cooldown countdown, deferred "fire", lifetime expiry -> destroy, enemy-in-region detection, mover script step, wake physics. |
| 0x100 hit wall | if S: FUN_00448cf2 (latch "bumped wall" mover event); wake. |
| 0x200 bump | edge test; other != 0; cooldown must be <= 0, then cooldown = 0x140000 (20 units ~ 0.24 s); optional FUN_004bf48c; if S: FUN_00448e8b (start script + latch bump/hit mover events). Remaining code is unreachable (see notes). |
| 0x40 tag / 0x80 trigger | (0x80 only: edge test first) tag-wait / ON T(tag) logic of the mover script (below). |
| others | ignored (0x02 shutdown not handled). |

**Fields used**

| rec | off | type | meaning |
|---|---|---|---|
| obj | +0x04 | u32 | contact bits (edge test), bit 0x20000000 destroyed |
| obj | +0x10 byte1 (+0x11) | bits | 0x04 mover script attached; 0x08 script running; 0x40 waiting for tag (`WT(t)`) ; also 0x10/0x20/0x80 other script wait states (text, key, anim) handled inside FUN_0044970d |
| obj | +0x12 | bits | 0x01 cleared after firing; 0x02 "listening for an ON T(tag) block" |
| obj | +0x13 | bits | 0x10 "watch for enemies in my region" (set at attach when script ON-conditions contain 0x440) |
| obj | +0x18 | ptr | home Point (created at init if missing) |
| obj | +0x20 | u32 | pending action word (output of the mover interpreter; 0x10000000 destroy; bit 2 = fire projectile) |
| obj | +0x44/+0x46 | 16.16 | tag; int part (+0x46) is re-used by the mover as the tag it currently waits/listens for |
| obj | +0xfc | ptr | current region (tick does nothing if 0 and no script = template objects) |
| obj | +0x118 | i32 16.16 | bump cooldown, set to 20.0 on accepted bump, decremented by frame time in tick |
| obj | +0x1b0 | ptr | aux: only sound loading here |
| obj | +0x1b4 | ptr | S, mover state block |
| S | +0x04 | u16 | pending mover events: 0x40 "event pending", 0x80 tag event, 0x10 bumped by player, 0x08 hit by player, 0x02 "currently inside an ON block"; +0x05: 0x01 lifetime enabled, 0x40 enemy seen, 0x80 bumped wall |
| S | +0x16 | i16 | tag of the ON T(tag) block being listened for (FUN_00448c58) |
| S | +0xc8 | i32 | parameter for player-control actions (FUN_00425256) |
| S | +0x110 | u32 | OR of all ON-block condition flags of the script (0x10 actor=player, 0x100 H hit, 0x200 B bump, 0x80000 W wall, 0x400000 K kill...) |
| S | +0x114 | i32 | index of the ON block currently running (-1 none) |
| S | +0x118..+0x124 | ptr[4] | the 4 ON blocks (each: u32 cond flags, +0x08 code ptr, +0x0c tag for T()) |
| S | +0x1d0 | u32 | expiry time (compared with game clock FUN_0049fbe4) |

**Pseudocode**

```
on 0x10:
  if self.region == null and !(flags2 & SCRIPT_ATTACHED): return
  if cooldown > 0: cooldown -= frame_dt
  if pending & 2:                      // deferred "fire" requested by the script
      launch_projectile(self, 0, 0)    // FUN_00464e68
      pending &= ~6; flags2 &= ~0x10000
  if S:
     if S.lifetime_enabled and now() > S.expiry: pending = DESTROY; wake(self); return
     if flags2 & SCRIPT_ATTACHED:
        if (flags2 & WATCH_ENEMY) and !S.in_on_block and enemy_in_region(self): S.events |= PENDING|ENEMY_SEEN
        if !(flags2 & SCRIPT_RUNNING): return          // attached but dormant: no physics wake
        a = mover_step(self)                            // FUN_0044970d
        if a & 0x20000000: apply_to_player(a, S+0xc8); pending = 0; return   // script drives the player
        pending = a
        if a == 0x4000000: return                       // "idle": no wake
  wake(self)

on 0x100: if S: mover_wall_bump(self, S, other); wake(self)

on 0x200:
  if contact or other==null or cooldown > 0: return
  cooldown = 20.0
  if (self+0x1d2 & 2) and is_player_projectile(other): FUN_004bf48c(self)
  if S: mover_bump(self, S, other)   // FUN_00448e8b
  return                             // (rest unreachable)

on 0x40, or 0x80 after edge test:
  if !(flags2 & WAIT_TAG):
      if flags2 & LISTEN_ON_T:                 // the listened tag fired
          S.events |= PENDING|TAG_EVENT; flags2 &= ~LISTEN_ON_T
          flags2 |= SCRIPT_RUNNING; tag_int = 0
  else:                                        // WT(t) satisfied
      tag_int = 0; flags2 &= ~WAIT_TAG
      if S: t = next_ON_T_tag(S)               // FUN_00448c58
            if t > 0: flags2 |= LISTEN_ON_T; tag_int = t
```

**Edge cases / behaviour**
* Bump is edge-triggered and additionally rate-limited by the 0.24 s cooldown; the cooldown only runs down in
  ticks (i.e. while in a region or with a script).
* The 0x80 trigger behaves exactly like a tag event (but needs the object to be out of contact).
* Deleted only when (a) the state-block lifetime expires (pending = 0x10000000) or (b) the mover script returns
  a destroy action (script `XD`, `XN(..)` replace etc. go through +0x20). OTHING itself never hides objects and
  has no hit points: damage/kill is counted in the mover block (`SH(n)` hits-to-kill counter S+0x1b0,
  decremented when a player projectile bumps it and some ON block has the K condition; the ON-condition
  matcher FUN_00448fcc then fires `ON HK`).
* A dormant attached script (`SCRIPT_ATTACHED` but not `RUNNING`) also suppresses wake -> the object does not
  move until bumped/tagged. Scripts that should run immediately get RUNNING at attach.
* Unreachable code: after FUN_00448e8b returns non-zero it has always set `SCRIPT_RUNNING`, and the handler
  returns when that bit is set; so the following block (team/domain checks via +0x1ac/+0x1ae, DAT_0056e69c,
  FUN_0045fe91 and a network message) can never run. Do not reimplement it.

**Data check (effect_dump, all 18 levels)**
* 707 OTHING: flags 0x01000030 (705) / 0x01000032 (2); +0x0c=+0x10=+0x1d0=0; tag -1 in all; +0x54 = 0; S
  index -1 in all (state blocks are runtime-only). Every object already has a home point (pt2).
* Names: DETHBOT# 447, DEADBOT# 162, XCHANGE# 52, PLAYTRN# 18, KEYBOT# 17, DEADKEY# 9, plus 'XCHANG2' and ''.
  705/707 names appear in the level's `INITMOVES` list and have a script in `[moveractioncodes]`; names repeat
  (e.g. 50x DETHBOT1 in LIPS05) and OSETMOVE applies the script to every object of that name
  (FUN_00423bde for-each-by-name). The two unlisted ones ('XCHANG2' typo in one level, unnamed) stay inert.
* DEADBOT*/DEADKEY* have region -1: templates, cloned by `XN("DEADBOT1")` when a DETHBOT dies. 162 have an aux:
  slot 4 sound (DROPE01/MWEP6/KLANG01/GLASS14/THUNDR12) and int0 = 400..408 (int3 = 50 on some). OTHING only
  loads these sounds; the ints are not read by OTHING (the pickup effect is in the script: `ON BP { XT(401) ... XD }`).
  Fits.

---------------------------------------------------------------------------------------------------------------

### OSPOT (index 61, FUN_004299b2, mask 0) - confidence H

**Events:** only 0x01 (called directly at init). Sets flags |= 0x200 (immovable) and +4 |= 0x3000 (hidden +
inactive: not drawn, skipped by collision and tick). Nothing else.

**Meaning:** an invisible named position marker. It is used **by name** as a destination: in the data every
OSPOT name is referenced from aux slot 9 of OTELE objects (teleport destination), e.g. `RBIRTH01` (respawn spot,
target of the DEATH01 teleporters in every level), UPSTAIRS, SWIMMER1/2, CHEIRAFT, OPENER01, HIGHER01, SMART1,
WARP01, GIANT, WHIZZER1, RBIRTH02..05. 34 objects, all flags 0x01000230, aux int0 = 32 (64 for GIANT) and a
step sound in slot 0 (not used by OSPOT itself; may be read by the teleport code). All referenced names exist.

---------------------------------------------------------------------------------------------------------------

### OSPAWN (index 62, FUN_004299f4, mask 0) - confidence H (handler), L (consumer)

Byte-identical behaviour to OSPOT: on 0x01 set flags 0x200 and +4 |= 0x3000 (hidden, inactive marker).
No code compares against this effect, and nothing in XRB aux slots references the names, so it is only
reachable by name lookup. Data: 17 objects, one per level LIPS01..17, named after the level ('LIPS05', ...;
LIPS14's is 'HNH29'), placed 64..208 units from the player start (Objects[0]) and next to RBIRTH01.
Probable use: arrival point when entering the level by name (the orb loader moves the player to the object named
in DAT_00515e78 = OSTRAP aux slot 8 "arrival object", FUN_0042d272), but all shipped OSTRAPs leave slot 8 empty,
so in practice OSPAWN is unused. Reimplement as an invisible named marker.

---------------------------------------------------------------------------------------------------------------

### OPATH (index 171, FUN_00429948, mask 0) - confidence H

**Events:** 0x01 only: flags |= 0x200; +0x0c |= 0x200000 (byte +0x0e bit 0x20 = "I am a path holder");
if aux: +0x110 (Path ref) = path found by name from aux slot 9 (FUN_00424a3c). Not hidden by the handler.
(Also called lazily by FUN_0045a549 if the path holder was not yet initialised.)

**Meaning:** lets a mover follow a named Path: an object whose link +0x198 points to an OPATH holder takes the
holder's path (FUN_0045a549: next waypoint = Point index from path+0x44 (first) / path+0xc4[node] (next), node
index kept in obj+0x0a). Used only by OFOLLOW in shipped data.

**Data:** 3 objects, all named OPATH1 (LIPS00/14/16), aux slot 9 = 'RAFTPATH' (exists in each level, 2 nodes);
the XRB already has +0x0c = 0x200000 and +0x110 = RAFTPATH index (editor pre-resolved), region -1.
Referenced only by RAFTMAN1 (OFOLLOW) slot 9. Fits.

---------------------------------------------------------------------------------------------------------------

### ODESTROY (index 78, FUN_0042684a, mask 0x2c1) - confidence H

**Events**
| ev | action |
|---|---|
| 0x01 | flags |= 0x200; load aux sounds |
| 0x40 tag | fire (no edge test) |
| 0x80 trigger | edge test, then fire |
| 0x200 bump | only if other is the player or player-owned projectile (FUN_0048a05a), edge test, then fire |

**Fire:** `t = find_object_by_name(aux.slot9)` (FUN_00424862, objects only); if t exists and is not already
destroyed: `t.pending = 0x10000000` (destroy next frame), wake(t), play aux sound 4 (aux+0xac) at `other`
(FUN_0049d933 positional sound). Repeatable; ODESTROY itself is never disabled, but the target can only be
destroyed once. No aux -> nothing.

**Fields:** aux+0x7a slot 9 target name; aux+0xac sound 4.

**Data:** 3 objects, all LIPS00 (tutorial): DESTNEED->NEEDGOLD, DESTROY2->BUMPWALL, DESTROY1->FINDLAMP; the
targets are OTEXT hint objects, and each ODESTROY is triggered by an OTIMER whose slot 9 names it (0x80).
I.e. "remove this hint text after a delay". Aux ints 0, no sound. Fits.

---------------------------------------------------------------------------------------------------------------

### OFOLLOW (index 10, FUN_0046038f, mask 0x3d0) - confidence M (H for init/path mode)

A moving object that follows a target object or a path, can carry another object along, can shoot at its
target, and can host a mover script (same tick/tag/wall/bump hooks as OTHING).

**Guard:** ignore everything if pending action has destroy (+0x23 & 0x10) or flags & 0x400.

**Init (0x01)**
```
self.point.type |= 3 ; +4 |= 0x10000 ; flags2 |= 0x08
if !aux: target(+0x198) = player
else:
  c = aux.int4 (+0x98): d1=(c/10)%10, d3=(c/1000)%10, d4=(c/10000)%10
  if d1: flags2 |= 0x100 ("idle / not yet captured")
  if d1 in {1,2} and region: region+6 |= 0x30           // region reports player enter/leave
  d3==1: +4 |= 0x100000 ; d3==2: +4 |= 0x80000
  target = find_object(aux.slot9) or player
  carried(+0x18c) = find_object(aux.slot6) ; if carried: carried.home(+0x18) = carried.point   (z offset ref)
  dest(+0x1bc) = find_point(aux.slot8) ; if dest: +4 |= 0x800 (path mode)
if target is an OPATH holder (+0x0e & 0x20): +0x0c |= 0x400 ; first = first path node (FUN_0045a549);
   dest = dest or first ; path mode
+0x124 = 0 (not dying); copy max speeds +0x7c..+0x88 to +0xcc..+0xd8
if d4 > 0: flags2 |= 0x100000 (shooter); S = new state block; S+0x134 = own tag, own tag = 0;
           S+0x136 = aux.int2 (+0x90), aux.int2 = 0
load aux sounds; create home point (+0x18) like OTHING
```

**Tick (0x10)** - only when in a region:
1. Carried object (+0x18c): placed at self x,y; z = self z + its home-point z; clamped below the region ceiling
   (ceil - height - 4); same angle; relinked into self's region; if the player is within (player r + carried r)
   (FUN_00468f3e distance) send 0x200 to the carried object's effect.
2. Deferred fire (+0x20 & 2): launch projectile; if in the player's region play projectile aux sound 0 + 1
   (or own aux sound 3 if the projectile has no aux).
3. S lifetime expiry -> destroy (same as OTHING); mover script attached: if running, step it into +0x20 and
   wake unless 0x4000000; if attached but not running, return.
4. Follow mode (not path mode, or idle): if target and not idle: dest defaults to target's point; if dying
   (+0x124==1) wait for the sprite animation to end (FUN_004a08dc) then destroy; else steer to dest (FUN_0045ac54:
   face dest, set forward bit 0x10 unless within radius, adjust z for flyers, advance along path nodes).
   No target or idle -> wander (FUN_00462d35 mode 3). Shooter: cooldown +0x120 counts down by frame time; at <=0
   reset to 200.0 and fire at once (with the same sounds). Within 5*radius of the target (x and y) -> stop
   (speeds 0, forward bit cleared).
   Path mode (and not idle): steer along the path; when a node is passed queue a network host update
   (FUN_0048993d).
5. wake.

**0x100 wall:** mover wall-bump latch; if no script is running: FUN_004630e5(self, wall, 3) (wall reaction).

**0x200 bump** (other != 0, edge test): mover bump hook; if no script running: r = FUN_0045fe91(self, other);
if r == 1: +0x124 = 1 (play out animation then destroy); r > 1: looks up aux slot 7 object and discards it (no-op).

**0x40 tag:** if waiting for tag: clear wait, tag = 0; else if listening for ON T: S.events |= 0xc0, tag = 0
(unlike OTHING it does not set SCRIPT_RUNNING nor re-arm the next listen tag).
**0x80:** in the mask but not handled.

**0x400000 / 0x800000 (player entered / left region):** code exists ("capture": entering actor passing the
units-digit filter FUN_004602ed becomes the new target when d1==2 or (d1==1 and idle), actor flags2 |= 0x200,
idle cleared, sounds 0/1; leaving: if hundreds digit set and actor is the target, become idle, stop sound 1),
but the region senders (FUN_004d81aa/FUN_004d8255) only dispatch these events when the effect mask has
0x400000/0x800000, and OFOLLOW's mask is 0x3d0 -> dead in practice.

**Data:** 3 objects, all RAFTMAN1 (LIPS00/14/16): flags 0x01000010, +0x0c 0x420, flags2 0x48, aux ints all 0,
slot 8 'PT1' (an existing named Point, start waypoint), slot 9 'OPATH1' (the OPATH holder). The XRB already has
+0x110 = RAFTPATH, +0x1bc = PT1, +0x198 = OPATH1 (pre-resolved). With d1..d4 = 0 and aux int1 = 0 the raft just
travels RAFTPATH (2 nodes) forever; bump does nothing (FUN_0045fe91 returns 0); no carried object, no shooting.

---------------------------------------------------------------------------------------------------------------

#### [A] Sub-functions decoded (reusable)

| addr | meaning |
|---|---|
| FUN_00424862(name) | find Object by name (exact compare, objects only; empty/placeholder name -> 0) |
| FUN_00424a3c(name) | find Path by name (path+0x0c) |
| FUN_00424911(name) | find Point by name (point+0x44) |
| FUN_00423bde(self,name,cb,..) | for-each object with that name -> cb (OSETMOVE applies to all same-named) |
| FUN_0049e0bd(aux) | resolve aux sound names slots 0..4 -> Sound refs aux+0x9c..+0xac (once; aux+4 bit0 = done) |
| FUN_0049d933(obj, snd, flag) | play sound positionally at obj (distance-gated vs player) |
| FUN_0049fbe4() | current game clock (ms-ish, GetTickCount based) |
| FUN_00467029(obj) | wake (known) |
| FUN_00464e68(obj, dang, pitch) | launch a projectile from obj (tool/projectile template by owner: player tools or S+0x154 bot tool), returns it |
| FUN_0044a1bb(obj, S, script) | attach compiled mover script: S+0xb0 script, ip, S+0x110 = OR of ON-conditions, S+0x118[4] ON blocks, flags2 \|= 0x0400 (SCRIPT_ATTACHED) and listen-tag setup, +0x13 bit 0x10 if conditions & 0x440 |
| FUN_0044970d(obj) | mover script step: first checks pending events -> jumps to the matching ON block (saving ip/wait/action to S+0xf4..+0x100), else runs IN block / waits / walk-to-point actions (0x800000 via FUN_0045a9d8, 0x400000 via FUN_0045a775) / next opcodes (FUN_00448388); returns the action word for obj+0x20 (0x4000000 = idle) |
| FUN_00449323(obj,S) | find first ON block (not the current one) whose condition matches (FUN_00448fcc) |
| FUN_00448fcc(obj,S,block) | ON-condition matcher: B/H by player (S+4 bits 0x10/0x08), net player (S+5 0x10/0x20), sight (FUN_00448872 range S+0x152), enemy in region, W wall, U"text", K kill (hits counter S+0x1b0), T(tag) (sets next WT) |
| FUN_00448cf2(obj,S,wall) | wall-hit latch: if obj flags&0x10, script has B+W condition, not in ON block -> S+5 \|= 0x80, S+4 \|= 0x40 |
| FUN_00448e8b(obj,S,other) | bump latch: needs other = player-ish or its projectile (FUN_00489f14) and obj flags&0x20 and script attached; sets SCRIPT_RUNNING, adds obj to running-mover lists; other==player and script has BP -> S+4 \|= 0x50; other = player projectile and script has H -> S+4 \|= 0x48; returns the actor (projectile owner or other) or 0 |
| FUN_00448c58(obj,S) | S+0x16 = tag of an ON T(tag) block (cond 0x8000) other than the current one |
| FUN_00425256(action, n) | mover action applied to the player: 0x100/0x200 -> player pending bits, 0x400/0x800 turn player left/right by n, 0x4000/0x8000 look up/down |
| FUN_004d94b2(obj, region) | first other object in region with team (+0x1ae) > 0, team id (+0x1ac int) <= DAT_0056e69c+0x34 and != own -> "enemy present" |
| FUN_00489f14(obj) | actor test: obj itself if player-ish, else owner if obj is a projectile (0x200000) of a player-ish owner, else 0 |
| FUN_00489f90(obj) | projectile (0x200000) with player-ish owner -> owner |
| FUN_0045fe91(self, other) | "touch reward" test: aux int1 code: units 1 = other must be the player, 2 = must be a projectile; tens = return value; with flags 0x1000 counts +0x54 uses (fires tag while >0, returns tens when used up); else fires own tag / S+0x132 / (code 3: aux int2 or S+0x134) and plays aux sound 3/4, pending \|= 0x820 for projectiles; requires player-ish actor and no contact |
| FUN_0045a549(obj) | next waypoint Point for path following (own path +0x110, or path of the OPATH holder at +0x198) |
| FUN_0045ac54(obj, 0) | steer toward obj+0x1bc: set angle, forward bit unless within radius, climb/descend for flyers (target z = floor + aux int0), advance to next path node; returns 1 when a node was passed |
| FUN_004d7317(obj) | allocate an object state block (S) for obj ("AddObjectState") |
| FUN_0042d272(obj) | move the player/camera object to obj (arrival after orb load) |

#### [A] Open questions
* FUN_004bf48c / FUN_004bdf98 (OTHING bump with +0x1d2 & 2 and a player projectile): uses obj+0x10c linked
  object with kind 2/3; not decoded (all shipped +0x1d0 = 0, so it never happens).
* Exact pending-action bits returned by the mover interpreter (+0x20: 0x10/0x20/0x800/0x100/0x200 etc.) belong
  to the mover/physics docs; only 0x10000000 (destroy), bit 2 (fire), 0x4000000 (idle), 0x20000000 (player
  control) are confirmed here.
* OSPAWN consumer: none found besides the generic by-name arrival lookup (DAT_00515e78), unused by shipped data.
* OFOLLOW 0x400000/0x800000 capture logic is masked out (0x3d0); maybe also reached through FUN_004489f6 for
  objects with +0x13 & 2 - not checked.
* Meaning of aux int0 400..408 on DEADBOT templates (one less than the ONTAG numbers the scripts fire); not read by OTHING.


---

<!-- part B -->

Notation: `self` = the object running the effect, `other` = EBX extra, `aux` = self+0x1b0, `int0..int4` =
aux+0x88..+0x98, `slotK` = aux name slot K (aux+0x0e+12K), `S0..S4` = aux+0x9c..+0xac Sound pointers.
"edge" = contact bits self+4 & 0x0f all clear (see brief). Times: `now100()` = FUN_0049fc30 = 100 ms tick
counter, `nowSec()` = FUN_0049fbe4 = seconds counter. `rand()` = FUN_004d61ec (= abs(GetTickCount + crt rand)).

Common trigger test (the same test is used by OPHYSON, OVIEWON, OVIEWOFF and OFLASH; written `TRIGGERED` below):
```
TRIGGERED(ev) = ev == 0x40                                   // tag: no edge test
             || ((ev == 0x80 || (ev == 0x200 && player_of(other) != 0))   // FUN_0048a05a
                 && edge(self))                              // edge test applies to 0x80 and 0x200
```

---

### OWALK (index 5, FUN_00463537, mask 0x350) - confidence M (core walk/bump H, mover hooks M, unused branches L)

Walking/flying monsters ("DETHBOTn" etc.). The handler does not move the object itself. Each tick it writes
the object's **action word +0x20** (the same bits as the player's input mask, physics.md section 3.1: 0x10 FORWARD,
0x20 BACKWARD, 0x100 UP, 0x200 DOWN, 0x400 TURNLEFT, 0x800 TURNRIGHT, 0x2 SHOOT, 0x10000000 DESTROY). It then
wakes the object (FUN_00467029), and the generic object physics moves it using the speed limits +0x7c..+0x98.

**Global guard (all events):** skip the event if self+0x20 has 0x10000000 set (destroy pending, byte +0x23 & 0x10)
or flag 0x400 is set (self+1 & 4).
Precomputed: `code = aux ? int1 : 0`, `tens = (code/10)%10`.

**Trigger events**
- `0x01 init` (direct call):
  - The position Point's byte0 is ORed with 3, flags10 |= 0x10, and +0x124 = 0.
  - If the Point has a point-path (Point+0x40), start it at segment 1 (FUN_004a9b29).
  - If there is an aux:
    - slot8 names a Point (FUN_00424911). If it exists: +0x1bc = that point, self+4 |= 0x800 (path-follow mode).
    - Resolve the sounds (FUN_0049e0bd).
    - int4 == 1 sets self+4 |= 0x100000. int4 == 2 sets self+4 |= 0x80000. These are wake conditions for an
      attached mover script (FUN_004489f6 condition 99): 0x100000 = the player's region "sees" the bot's home
      region (FUN_004d92a0), 0x80000 = the player is in the same region.
    - slot6 names an object, the "rider/carried" object, stored at +0x18c. If that object has no +0x18 point, one
      is allocated, but its +0x18 is then set to its own +0x14 point (see the quirk below).
  - Copy +0x7c,+0x80,+0x84,+0x88 (base max speeds) to +0xcc..+0xd8 as a saved copy.
  - If +0x18 (home Point) is null: allocate a new Point (FUN_004d6d98), copy the position point into it (100
    bytes), keep the new point's own index, and store the current angle in its +0x34 as angle<<16. Shipped
    data already has a +0x18 point for every OWALK.
- `0x10 tick` (active list). Runs only if self+0xfc (region) != 0:
  1. **Rider (+0x18c)**, if it is set and has a different Point:
     - rider.xy = self.xy.
     - rider.z = self.z + rider.home.z. Clamp it so that rider.z + rider.height <= region ceiling - 4.
     - rider.angle = self.angle. Re-link the rider into self's region if it differs (FUN_004d88ab/FUN_004d85a1).
     - If the 2D distance(player, rider) < player.r + rider.r, dispatch **0x200** to the rider's effect, with
       extra = the player.
  2. **Shoot**: if +0x20 & 2 (set by mover scripts, never by the built-in AI):
     - p = FUN_00464e68(self,0,0) (spawn a projectile at radius+4 ahead). Clear +0x20 bits 2|4.
     - If self is in the player's region and p != 0: if p has an aux, play p.S0 (mode 1) and p.S1 (mode 0).
       Otherwise play self.S3 (mode 1).
     - Clear flags10 bit 0x10000.
  3. **Mover script** (+0x1b4 state block present):
     - Lifetime: if state+5 & 1 and nowSec() > state+0x1d0, then +0x20 = DESTROY, wake, return.
     - If flags10 & 0x400 (a script is attached):
       - If flags10 & 0x800 is clear (script not yet activated), return and do nothing this tick (the bot
         stands still).
       - Otherwise +0x20 = FUN_0044970d(self) (the script interpreter's action word for this frame). If it is
         not 0x4000000 ("no script action"), wake and return. Otherwise fall through to the built-in AI.
  4. **Built-in movement**:
     - If self+4 & 0x800 (path mode): FUN_0045ac54(self,0) follows the Point chain (see sub-functions), then wake.
     - Else if the Point has a point-path: FUN_004aa1c4 steps it. If it returns 2 (end), restart at segment 1.
       No wake on this branch.
     - Else: FUN_00462d35(self,0), the random walk below, then wake.
- `0x40 tag`: **only does something for mover-scripted bots**:
  - If flags10 & 0x4000:
    - Clear it. Then, using the state block, look up the tag of the script's `T(tag)` ON-condition
      (FUN_00448c58 -> state+0x16).
    - If that tag > 0: flags10 |= 0x20000 and the object's tag int (+0x46) = that tag, so the object now listens
      for it.
    - Bug: in this branch the state pointer is an **uninitialised stack variable** (ebp-8). It is clearly meant
      to be self+0x1b4.
  - Else if flags10 & 0x20000 and a state exists: state+4 |= 0xc0 (signal that the T-condition fired), clear
    0x20000, and the tag int = 0.
  - A plain OWALK without a script ignores tag activation.
- `0x100 wall hit` (other = the wall):
  - If a state exists: FUN_00448cf2 may set the "hit wall" script condition (state+5 |= 0x80, state+4 |= 0x40).
  - If no script is active (flags10 & 0x800 clear), or the script returned "no action" (+0x20 & 0x4000000):
    bounce with FUN_004630e5(self, wall, 0):
    - The new heading is the mirror reflection about the wall line, `2*wallAngle - heading` (mod 2048),
      where wallAngle = atan2(wall.p1 - wall.p2) (FUN_0045e80a). It is not applied if the result is exactly 0.
    - +0x20 |= 0x110 (FORWARD|UP).
    - Then rand%3: 1 = TURNRIGHT, 2 = TURNLEFT, 0 = none.
    - If the object carries a region (flags0c & 0x20, +0x100), rotate that region by the change in heading
      (FUN_0044eefd).
- `0x200 bump` (other != 0, edge test on bits 1,2,4,8):
  - If +0x1d0 & 0x20000 and other is a projectile with a player-ish owner (FUN_00489f90): FUN_004bf48c(self)
    (overlay +0x10c update, probably a hit/health indicator; L).
  - If a state exists: FUN_00448e8b sets the script conditions. A valid bumper/hitter that passes FUN_00489f14,
    on a scripted bot (flags & 0x20 and flags10 & 0x400), **activates the script** (flags10 |= 0x800 and adds
    the bot to the mover lists). It also sets "BP" (bumped by player: state+4 |= 0x10|0x40) or "HP" (hit by a
    player projectile: state+4 |= 0x08|0x40), depending on state+0x110/+0x111 enables.
  - If no script is active (flags10 & 0x800 clear): r = FUN_0045fe91(self, other) (see sub-functions). If r == 1
    (`tens` digit is 1, a killing hit):
    - +0x124 = 1.
    - No aux: if tens == 1, +0x20 = DESTROY. Wake.
    - With aux:
      - If no object named slot7 exists: if tens == 1, +0x20 = DESTROY, then wake. If slot7 does name an
        existing object, nothing at all happens to self: no destroy and no wake.
      - Then ActivateTag(int3), or ActivateTag(state+0x136>>16) when a state block exists.
- Not handled: 0x08, 0x80, 0x400 and others.

**Random walk FUN_00462d35(self, mode 0)** (each tick):
```
if aux && int0 != 0: angle = (angle + int0) wrapped to 0..2048      // constant curve, raw angle units per tick
c = self+0x128                                  // phase counter, 0 in all shipped data
if c > 0: c -= 1; if c == 0: c = -(rand() % fps)  // fps = DAT_00515ce8 (1 if fps<1)
          action |= (rand()%2==1) ? TURNRIGHT : TURNLEFT      // random jitter turn every tick
if c < 1: c += 1; if c >= 0: c = rand() % fps    // the next turning phase starts
=> alternating phases: "turning" (c>0, a random left/right each tick) and "straight" (c<=0), each up to ~1 s long
action |= FORWARD
if flags & 2 (flying):
    if vz<0 and z > floor+4        -> DOWN
    if vz>0 and z < ceiling-radius -> UP
    if vz==0 and TURNLEFT set -> UP ;  if vz==0 and TURNRIGHT set -> DOWN
if flags0c&0x20 && +0x100 region: rotate that region by int0 (FUN_0044eefd)
if flags&0x40 and int0: rotate the Point chain hanging off the position point (Point+0x3c links) by int0 (FUN_004aa5d5)
mask out actions whose max speed is 0: +0x7c==0 -> clear FWD/BACK; +0x80==0 -> clear strafe L/R;
    +0x84==0 -> clear UP/DOWN; +0x88==0 -> clear TURNL/TURNR
```
Modes 1..4 of FUN_00462d35: 1 = only FORWARD; 2..4 = action = FORWARD only, clearing everything else (other
callers).

**Fields used**

| rec | off | type | meaning |
|---|---|---|---|
| obj | +0x00 | flags | 0x2 fly, 0x20 collides, 0x40 rotate point chain, 0x1000 (byte1&0x10) limited hit counter, 0x400 skip (OORIENT-attached) |
| obj | +0x04 | rt bits | contact bits 0..3 (edge); 0x800 path mode; 0x80000/0x100000 mover wake condition |
| obj | +0x0c | flags | 0x20: carries the region at +0x100 |
| obj | +0x10 | flags10 | 0x10 set at init; 0x400 script attached; 0x800 script active; 0x4000/0x20000 T(tag) wait; 0x10000 cleared after a shot |
| obj | +0x14/+0x18 | Point | position / home point (spawn position + angle) |
| obj | +0x20 | u32 | action word written each tick |
| obj | +0x24 | u16 | heading (2048/turn) |
| obj | +0x3c | 16.16 | radius (path arrival, rider bump test) |
| obj | +0x44 | 16.16 | tag fired on hits (FUN_0045fe91) / for scripts, the tag listened to |
| obj | +0x54 | i32 | hits/bumps counter when flag 0x1000 (4 in 305/307 shipped) |
| obj | +0x7c,+0x80,+0x84,+0x88 | 16.16 | max forward, strafe, vertical, turn speed (a zero value disables that action) |
| obj | +0x8c,+0x90,+0x94 | 16.16 | accelerations (physics), +0x98 unknown (2275.0 in almost all) |
| obj | +0xb4 | 16.16 | vz (flying logic) |
| obj | +0xcc..+0xd8 | 16.16 | saved copy of +0x7c..+0x88 |
| obj | +0xfc | Region | required for the tick (floor +0xc, ceiling +0x10) |
| obj | +0x124 | i32 | 0 at init, 1 after a killing bump/hit |
| obj | +0x128 | i32 | random-walk phase counter |
| obj | +0x18c | Object | rider (aux slot6) |
| obj | +0x1b4 | State | mover script state (INITMOVES/OSETMOVE) |
| obj | +0x1bc | Point | current path target (aux slot8, then Point+0x3c chain) |
| obj | +0x1d0 | flags | 0x20000: overlay hit indicator |
| aux | int0 | i32 | constant turn per tick (random walk); path-mode altitude offset with +0x198 |
| aux | int1 | i32 | hit code: units 1 = only player bodies count, 2 = only projectiles (flag 0x200000) count, other = any; tens 1 = a hit kills; ==3 exactly: projectile hit fires int2 instead of own tag |
| aux | int2 / int3 | i32 | tag for projectile hits (code 3) / tag fired after a kill |
| aux | int4 | i32 | mover wake condition (1 = region visible, 2 = same region) |
| aux | slot6/7/8 | name | rider object / "survive-if-exists" object / first path Point |
| aux | S3 / S4 | Sound | body-bump sound / projectile-hit sound; S3 is also the shot sound fallback |

**Edge cases**
- Plain bots never die from bumps unless an aux has tens==1. With flag 0x1000 (all shipped bots), each
  player bump or player projectile hit:
  - fires the own tag while +0x54 > 0, then decrements +0x54, plays S3 and recoils (+0x20 |= BACKWARD|TURNRIGHT);
  - when +0x54 reaches 0 it becomes -1 and returns `tens` (a kill only if tens==1); later it keeps decreasing.
- Only player-related bumpers count (FUN_0045fe91 returns 0 unless FUN_0048a05a(other) != 0).
- Quirk in the rider z: its home point *is* its position point, so the rider's z gets self.z added every tick and
  sticks at the ceiling clamp unless self.z == 0. None are shipped (no slot6).
- Path mode always steers the altitude toward `local_20`, which is 0 unless +0x198 is set, and it forces the
  fly flag (L, no shipped use).

**Data check** (307 objects, 16 levels):
- All have flags 0x01001030 (19 add fly 0x2, 6 add 0x6), f10=0x10, +0x54=4 (2 others 5/9), and a +0x18 home point.
- No state, rider, path, point-path or region refs in the file.
- Names: 233 are DETHBOT1..9 (scripted at runtime via `INITMOVES=(DETHBOT1..9)` in the INIs, so the built-in
  AI runs only as the "no action" fallback), 74 unnamed, plus OSCAR1/2.
- Typical params (msx,msy,msz,mst | ax,ay,az | +0x98):
  - 400,400,200,853 | 400×3 | 2275 (52)
  - 800,800,150,853 (43)
  - 200,200,200,455/568/284/398 (~125)
  - 60,60,0,227 with +0x98 = 17066 (10)
  - The turn values match degrees*512/90 (853 = 150°, 568 = 100°, 455 = 80°, 284 = 50°, 227 = 40°).
- Aux: 259 have none. 47 have only int4 = 1 or 2 (wake condition; 1 is mostly on flyers). One (LIPS14 #116) has
  int1 = 3 (no filter, tens 0), int2 = 41 and S4 = SPLAT03. It also has flag 0x1000, so the code==3 path is not
  reached for it.
- Tagged plain bots: LIPS14 tag 3 (×3), LIPS12 tag 50 (×3), LIPS00 105/71. These fire their tag on each of the
  first 4 hits, which fits.
- Aux int0 = 0 everywhere, so no curving walkers ship.

---

### OORIENT (index 69, FUN_0046423b, mask 0x18) - confidence H

Glues an object's facing and height to a "leader" object. In shipped data the two objects **share the same
Point**, so x/y follow automatically.
- `0x01 init` (aux required):
  - leader = the object named slot9, stored at +0x198.
  - If it exists: flags |= 0x400 (makes OWALK-style handlers and object collision skip it). Then flags |= 0x8000
    if +0x54 == 0, else flags |= 0x4000 (meaning not traced). Resolve the sounds.
- `0x08 post-frame tick`, if a leader exists:
  - If the leader is not destroy-pending (leader+0x20 & 0x10000000 clear):
    - self.angle = leader.angle + (short)int0.
    - self.z (+0x2c) = leader.z, and self+0x30 = leader+0x30.
  - Otherwise self+0x20 |= 0x10000000 (destroy with the leader).
  - Then always step the sprite animation (FUN_004a09fe(self, self+0x64)).
- 0x10 is in the mask but ignored. There is no destroyed-guard.

| rec | off | meaning |
|---|---|---|
| aux | slot9 | leader object name |
| aux | int0 | angle offset added to the leader angle (raw 2048/turn units) |
| obj | +0x54 | selects flag 0x8000 (0) or 0x4000 (!=0) |
| obj | +0x198 | leader pointer (also in the file as an object index) |

Data: 10 objects in LIPS13. Flags are 0x01008430 (0x400|0x8000 already set), int0 = 0, +0x54 = 0, and slot9 =
B1..B10. All leaders exist; they are **OTRANSIT** objects with the identical Point index, z and angle (a moving or
rotating group). This fits.

---

### OPHYSON (index 172, FUN_00438061, mask 0x2c0) - confidence H
- `0x01`: flags |= 0x200 (immovable), and resolve the aux sounds if there is an aux.
- When TRIGGERED: **movement model DAT_00510774 = self+0x54**. Then, if there is an aux, play S4 with
  other as the source (FUN_0049d933(other,S4,1)).
- Repeatable, no state. (The adjacent FUN_00438171 = OPHYSOFF sets the model to 0.)

Data: 4 objects (LIPS00 ×2, LIPS01, LIPS06). All have tag **9999** (the startup broadcast fired at every
level (re)start), +0x54 = 0 and no aux. So they force model M = 0 at start. This confirms the physics.md
assumption: OPHYSON is used, through tag 9999 rather than by name.

---

### OVIEWON (index 181, FUN_00437702, mask 0x2c0) - confidence H
- `0x01`: flags |= 0x200, and resolve the aux sounds.
- When TRIGGERED (and there is an aux): view = the View named slot9 (FUN_00456d87, matched on View+0x5c). If
  found, set view+0 |= 2 (view enabled/rendered). Play S4 via FUN_0049d933(other,S4,1).

### OVIEWOFF (index 182, FUN_00437820, mask 0x2c0) - confidence H
- `0x01`: as OVIEWON.
- When TRIGGERED and the slot9 view exists:
  - view+0 &= ~2 (disable).
  - If int0 > 0 and the view has a pixel buffer (view+0x14): fill width×height bytes (u16 view+0x10 × u16
    view+0x12) with palette index int0 (blank the screen surface).
  - Play S4.

| rec | off | meaning |
|---|---|---|
| aux | slot9 | View name |
| aux | int0 | (OFF) blank colour; 0 = leave the last image |
| aux | S4 | switch sound |
| view | +0x00 bit 1 | enabled (main view = 3, TV view = 2 in files) |

Data:
- OVIEWON: LIPS00 has 4 (tags 52, 53, 81, 9999); LIPS12 has 1 (tag 301).
- OVIEWOFF: LIPS00 has 1 (tag 100); LIPS12 has 1 (tag 302).
- All use slot9 = 'TVVIEW'. The View 'TVVIEW' exists in both levels (view #1, flags 0x2). (An object named
  TVVIEW also exists; the lookup is in the View table.)
- int0 = 0 everywhere, so no blanking. No S4.
- The LIPS00 tag-9999 OVIEWON switches the TV on at start.

---

### OFLASH (index 141, FUN_004307c3, mask 0x802d0) - confidence H

Lightning / strobe. It toggles the global **fullbright flag DAT_0051081c** (renderer.md section 7: every surface
then uses light DAT_0054868c).
- Guard: skip the event if self+4 & 0x20000000 (destroyed).
- `0x01`:
  - flags |= 0x200.
  - +0x118 = +0x11c = +0x120 = 0.
  - If there is an aux: int0 = 0 becomes 1, int1 = 0 becomes 1, +0x54 = 0 becomes 1. Resolve the sounds.
- When TRIGGERED (and there is an aux):
  ```
  flashes_left(+0x120) = +0x54 > 0 ? +0x54 : rand() % |+0x54|
  on_until(+0x118)     = now100() + (int0 > 0 ? int0 : rand() % |int0|)
  fullbright = 1;  play S4 non-positionally (FUN_0049d6c7(S4,1))
  ```
- `0x10 tick` (active list, aux required):
  ```
  if on_until > 0:
      if now100() < on_until: return
      fullbright = 0; on_until = 0
      if flashes_left < 1: return
      flashes_left -= 1
      off_until(+0x11c) = now100() + (int1 > 0 ? int1 : rand() % |int1|)
  if off_until > 0 and now100() >= off_until:
      off_until = 0
      on_until = now100() + (int0 > 0 ? int0 : rand() % |int0|)
      fullbright = 1
  ```
  So one trigger gives 1 + flashes_left flashes. int0 is the on-time and int1 the gap, both in 100 ms units;
  a negative value means random 0..|v|-1.
- The comparisons are unsigned against absolute 100 ms counters. Triggering again restarts the sequence. 0x80000
  in the mask is not handled.

| rec | off | meaning |
|---|---|---|
| obj | +0x54 | extra flash count (negative = random 0..|n|-1) |
| obj | +0x118/+0x11c/+0x120 | runtime on-until / off-until / remaining flashes |
| aux | int0 / int1 | flash on-time / gap (100 ms units; <0 = random) |
| aux | S4 | thunder sound |

Data:
- LIPS00 FLASH1 (tag 98): +0x54 = -5, int0 = 1, int1 = -5, S4 = THUNDR12. That is 1-5 flashes of 0.1 s with
  0-0.4 s gaps, plus thunder.
- LIPS09 FLASH01 (tag 28): +0x54 = 60, int0 = 1, int1 = 1, no sound. That is a ~12 s strobe of 61 flashes.
- Both fit.

---

#### [B] Sub-functions decoded (reusable)

| addr | meaning |
|---|---|
| FUN_0049fc30 | now in 100 ms ticks (DAT_006cc47c) |
| FUN_0049fbe4 | now in seconds (DAT_006cc46c) |
| FUN_004d61ec | rand(): abs(GetTickCount() + crt_rand()) |
| FUN_004eff5c | abs(int) |
| FUN_00424862(name) | find **Object** by name (+0x58) |
| FUN_00424911(name) | find **Point** by name (+0x44) |
| FUN_00456d87(name) | find **View** by name (+0x5c) |
| FUN_004d6d98() | allocate a new runtime Point ("AddPoint") |
| FUN_0049e0bd(aux) | resolve the aux sound slots 0..4 (names at +0x0e..+0x3e) to Sound ptrs +0x9c..+0xac, once (sets aux+4 bit0) |
| FUN_0049d6c7(sound, mode) | play a sound non-positionally (MIDI or wave; mode<1 seems to mean loop) |
| FUN_0049d933(srcObj, sound, mode) | play a sound with a distance cull: if src is a non-player object and the sound options DAT_0056e8e0+0x1c&1 are on, it plays only if dist(player,src) <= DAT_00567104; else plays it like FUN_0049d6c7 |
| FUN_004ef3a2(buf, byte, n) | memset |
| FUN_004a09fe(obj, tc) | sprite/texture animation step |
| FUN_00464e68(obj, dAngle, ?) | spawn the object's projectile (weapon from state+0x154 or default DAT_00543d00) at radius+4 ahead; returns the projectile or 0 |
| FUN_00462d35(obj, mode) | built-in random-walk AI (above) |
| FUN_004630e5(obj, wall, mode) | wall reaction; mode 0 = reflect the heading off the wall + random turn (modes 1,2: turn along the wall with 0x820/0x420 actions) |
| FUN_0045e80a(obj, wall) | reflected heading 2*wallAngle - heading |
| FUN_0045fe91(obj, other) | bump/hit evaluation for walkers (aux int1 code, +0x54 counter, sounds S3/S4, tags); returns the tens digit when the hit counts as a kill |
| FUN_0045ac54(obj, 0) | follow the Point chain from +0x1bc: face the target, FORWARD until within radius in x and y, then next = Point+0x3c (or a Path node via FUN_0045a549 if +0x110); clears path mode at the end; altitude steering |
| FUN_004aa5d5(obj, dA) | rotate the Point chain linked from the position point about it by dA |
| FUN_004a9b29(point, seg) / FUN_004aa1c4(point) | point-path (keyframed Point motion, Point+0x40) start segment / step (returns 2 at the end) |
| FUN_00448c58(obj, state) | state+0x16 = the T(tag) condition tag of the script's ON-blocks |
| FUN_00448cf2 / FUN_00448e8b | set the mover-script wall-hit / bump (BP, HP) condition bits; e8b also activates the script (flags10 \|= 0x800) |
| FUN_0044970d(obj) | run the mover script for this frame, returning an action word (0x4000000 = none) |
| FUN_004489f6 | mover wake condition (99: via obj+4 0x100000 / 0x80000, set from OWALK aux int4) |
| FUN_00468f3e(x1,y1,x2,y2) | 2D distance |
| FUN_00468d05(dx,dy) | atan2, angle in the high word |


---

<!-- part C -->

Conventions as in BRIEF.md. "edge test" = return if self.+4 & (1|2|4|8) (already in contact).
"active" bit for the sound effects = object +4 bit 0x4000 (byte +5 & 0x40).
Time sources (FUN_0049fa1c, driven by GetTickCount): `now_s()` = FUN_0049fbe4 = whole seconds
counter (DAT_006cc46c); `now_ds()` = FUN_0049fc30 = tenths of a second (DAT_006cc47c);
FUN_0049fc7c = 1/25 s units (DAT_006cc478).

---------------------------------------------------------------------------------------------

### OTEXT (index 132, FUN_0043e612, mask 0x802d0) – confidence H (kind 0/4/5), M (kinds 1/2/3)

On-screen message attached to an object. The handler only arms/disarms it; the drawing happens
every rendered frame in **FUN_0043d8d4** (called from the view render FUN_004aae49), which walks
the global OTEXT list.

**Events**
| event | action |
|---|---|
| 0x01 init (direct) | needs aux. Append self to the OTEXT list (DAT_0052f618[], count DAT_005100a4, max 512 → log "MAXOTEXTOBJECTS reached" and stop). Hide (clear +6 bit 0x04). Flags |= 0x200 (immovable). If aux+0x94 < 1 → 5. In net games (DAT_00510df0, game type = DAT_0056e8e0+0x3c): if aux y > 115 move it up 16 px (type 1,2) or 32 px (type 3). +0x11c = aux+0x98 % 1000 (remaining shows). Resolve aux sounds (FUN_0049e0bd). Kind 1/2: load a text resource (below), on success set +0x0c bit 8 and +0x1a8 = text record. Kind 3: +0x1a4 = object named in aux slot 9 (FUN_00424862). |
| 0x10 tick | if visible and now_s() > +0x118 → hide. |
| 0x40 tag | show (no edge test). |
| 0x80 trigger | edge test, then show. |
| 0x200 bump | other must pass FUN_00489f14 (player or player-owned projectile), edge test, show. |
| 0x80000 UI key | show (no test). |
| 2, 0x100000, 0x200000 | not handled (0x100000/0x200000 are not in the mask anyway). |

**Show** (shared tail):
```
if aux == null or visible: return            // re-trigger while shown does nothing (no extend)
limited = (aux.repeat % 1000) != 0
if !textfile && limited { if self.shows_left < 1 return; self.shows_left -= 1 }
self.expire_s = now_s() + aux.duration; visible = true
hide_same_line(self)      // FUN_0043e551: every other OTEXT whose aux.y <= self.y <= other.y+8 is hidden
self.+4 &= ~0x200
if textfile {             // kinds 1/2 that loaded
    if !advance_section(text) {           // FUN_0043e252: next "[...]" section after the current one
        if limited { if shows_left < 1 return; shows_left -= 1 }   // one "wrap" consumed
        rewind(text); if !advance_section(text) return
    }
}
play(aux.sound[0], once)  // FUN_0049d6c7(aux+0x9c, 1)
```

**Drawing** (FUN_0043d8d4(view, clear_all)), for each listed OTEXT:
* `clear_all` (=1 from the PageDown key handler and FUN_00479347) hides everything.
* hides it when now_s() > +0x118 (duration granularity 1 s).
* text source by object +0x54 (kind):
  * kind 0 (and any other value, incl. shipped 1/2/30/50 without a text file): concatenation of
    aux slots 5..9 (+0x4a,+0x56,+0x62,+0x6e,+0x7a) with `_` → space (FUN_0043d48d). Max 5×11 chars.
  * 4: `[namedmessages]` lookup of aux slot 5 (+0x4a) (FUN_0046e93e); if missing, the key itself.
  * 5: string at object +0x1a4 (≤128 chars) – used by the script `OTEXT("...")` command
    (FUN_0040d1ea creates such objects; default object `_DEF_OT_`, aux y=170, duration 5).
  * text file (kinds 1/2 loaded) or kind 3: multi-line: every line of the current section is drawn,
    13 px apart, until the next `[` line. Kind 3 = "OTMASTER": the text record comes from the master
    object (+0x1a4)'s +0x1a8, section = `[<aux slot 8>]` (FUN_0043d707); cursor restored after.
* render mode/colour come from **globals set by OTEXTSET** (not from the OTEXT record):
  DAT_005100b8 mode 0 (default) → FUN_00475bb4(view, aux.x + DAT_005100b0, aux.y + DAT_005100b4,
  fg DAT_005100a8 (default 176), bg DAT_005100ac (default 0 → a black 8·len+6 × 12 box is filled
  first; bg < 0 = transparent)); mode 1 → status-line message FUN_004b149e(text, aux+0x94 seconds,
  min 3).
* `aux+0x90` (32 in 294/302 records) is **never read** by OTEXT. `(aux+0x98 /1000)%1000` is computed
  in the drawer but unused.

Text resource loader FUN_0043dcb6(name = aux slot 9, mode = kind, …): name empty/"NO_NAME" → none.
Mode 1: file `<DAT_0070d424+0x334><name>.TXT` read whole. Mode 2: `GetPrivateProfileSection(name)`
from the INI DAT_00708368 (NUL-separated lines → '\n'). Builds a Text record (108 B, FUN_004d7af1
AddText): +0x0c name, +0x3c length, +0x40 alloc size, +0x44 buffer, +0x48 end, +0x4c line-pointer
table, +0x50 line count, +0x54 current section header line (−1), +0x58 parse cursor, +0x5c/+0x60
(lines, max width of the section, FUN_0043d631), +0x64 current line index (−1), +0x68 current line ptr.
Error string "Can't find OTEXT/OTMASTER Text data:".

**Fields**
| rec | off | type | meaning |
|---|---|---|---|
| obj | +0x54 | i32 | kind (0 slots, 1 .TXT file, 2 INI section, 3 OTMASTER section, 4 namedmessage, 5 script string) |
| obj | +0x06 bit 0x04 | | visible |
| obj | +0x0c bit 8 | | has loaded text file |
| obj | +0x118 | u32 | expiry, seconds |
| obj | +0x11c | i32 | remaining shows (limited mode) |
| obj | +0x1a4 | ptr | kind 3: master object; kind 5: string buffer |
| obj | +0x1a8 | ptr | Text record (kinds 1/2) |
| aux | +0x88/+0x8c | i32 | screen x / y (320×200) |
| aux | +0x90 | i32 | unused (colour-like value 32) |
| aux | +0x94 | i32 | display seconds (≤0 → 5) |
| aux | +0x98 | i32 | %1000 = max number of shows (0 = unlimited) |
| aux | slot 0 / +0x9c | | sound played on show |
| aux | slots 5–9 | char[12] | message text (kind 0), key (kind 4 slot 5), file/section/master names (slots 8/9) |

**Data check** (302 objects, all levels): aux ints x=0 always; y 158 (201), 138 (74), 160 (17),
118 (6), 0 (4); +0x90 = 32 (294) else 0/3/4/16/30; duration 5 in all; repeat 0 (277), 1 (24), 5 (1).
Kinds: 0 (284), 2 (11, LIPS07), 1 (1, LIPS08), 30 (3), 50 (3). The kind-1/2 objects have the text
in slots 5–9 and slot 9 is either empty or the tail of the message (e.g. "IFE!") → loader fails →
they render exactly like kind 0. The **Texts section is empty (count 0) in all 19 XRBs**, +0x1a8 = −1
everywhere; so shipped data only ever uses kind 0 (plus kind 5 from scripts). Longest messages 40
chars (8 chars used per slot) – fits 320 px (8 px glyphs). Sound slot 0 names all resolve (S0 shown).
The XRB "ObjTable512" block (DAT_0052f618) is a precomputed OTEXT list (indices of OTEXT objects in
order; −1 padded), count restored at load.

### OTEXTSET (index 133, LAB_0043e3fc, mask 0x2c0) – H (related, unused in data)
Event 1: flags |= 0x200, resolve sounds. On 0x40, 0x80 (edge), 0x200 (FUN_0048a05a player-ish, edge):
copies aux+0x88/+0x8c → global OTEXT x/y offset, +0x90 → fg colour, +0x94 → bg colour, +0x98 → mode
(0 draw at position, 1 status line). No objects use it in shipped levels.

---------------------------------------------------------------------------------------------

### OSOUND (index 53, FUN_0042e442, mask 0x3002d0) – confidence H (control flow), M (pan/volume units)

A (optionally positional / repeating / delayed) sound source playing aux sound slot 0.
Everything is ignored if the object is destroyed (+4 bit 0x20000000).

**Events**
| event | action |
|---|---|
| 0x01 init | flags |= 0x200; +0x0c |= 1 (marks "OSOUND" for FUN_0049d411/FUN_0049d666); if +0x54 == 0 also +0x0c |= 0x10. With aux: resolve sounds; volume +0x124 = clamp(aux+0x8c, ≤1000), ≤0 → 1000; +0x11c = +0x128 = +0x120 = 0; Sound(slot0)+0x44 = 0x8000. |
| 0x04 stop (direct, from FUN_0049d489 when a stopped Sound is an object proxy) | stop slot-0 sound, inactive, +0x11c = 0. |
| 0x10 tick | see below. |
| 0x40 tag | start (no edge test). |
| 0x80 trigger | edge test, start. |
| 0x200 bump | if +0x54 == 0: other must be the player (FUN_00489ed4) and FUN_00489f14; if active and (still playing and restart digit == 0) → ignore, else stop & reset; +0x120 = dist(other,self) + 2.0; listener for the initial volume = other; edge test; start. |
| 0x100000 (OEFFOFF) | if active **and still playing**: stop, inactive, +0x11c = +0x128 = 0 (a sound waiting in a delay is not cancelled). |
| 0x200000 (OEFFON) | in mask but not handled (only the dispatcher's enable bit matters). |

`aux+0x90` is split in decimal digits: units **P** (>0 = positional: pan + distance attenuation),
tens **R** (≠0 = re-trigger restarts a playing sound), hundreds unused.

**Start**
```
if active { if still_playing(snd) && R == 0 { return } ; stop(snd); inactive; delay=0; left=0 }
if !(snd.flags & 1) {                       // not the MIDI/CD type
    if P == 0 { set_volume(snd, vol) }
    else { set_pan(snd, pan_of(self)); set_volume(snd, vol - dist(listener,self)/max(aux.falloff,1)) }
}
left = aux.count                             // +0x11c
if aux.delay == 0 {
    play(src=other, snd, loop = (aux.count == -1))
    if aux.count > 0 && aux.count != 9999 { left -= 1 }
} else if aux.delay < 0 { next_ds = now_ds() + rand() % |aux.delay| }
else                    { next_ds = now_ds() + aux.delay }
active = true
```
**Tick** (only while active): if +0xfc (region/mover) ≠ 0 → wake (FUN_00467029, i.e. the sound
object can ride a mover).
```
if aux.delay != 0 && next_ds >= 1 {         // waiting
    if now_ds() > next_ds { play(snd, once); next_ds = 0 }
    return
}
if finished(snd) {
    if left == 0 { stop; inactive; next_ds = 0; return }
    if aux.count != -1 && aux.count != 9999 { left -= 1 }
    if aux.delay == 0 { play(snd, once) }
    else if aux.delay > 0 { next_ds = now_ds() + aux.delay }
    else { next_ds = now_ds() + vol }        // QUIRK: uses +0x124 (volume) as the delay, not random
}
if P > 0 {
    d = dist(player DAT_00510778, self)
    if d > self.+0x120 { stop; inactive; next_ds = 0; +0x120 = 0 }   // walked away from the bump point
    else { set_pan(snd, pan_of(self)); set_volume(snd, vol - (d>>16)/max(aux.falloff,1)) }
}
```
So total plays = max(1, aux.count) (0 → once), −1 = hardware loop forever, 9999 = re-trigger forever.

**Fields**
| rec | off | meaning |
|---|---|---|
| obj | +0x54 | 0 = only the player may bump-start it; ≠0 = any bumper (shipped 2/4/5 on 10 objects) |
| obj | +0x11c | plays left; +0x120 max listener distance (16.16, set on bump); +0x124 volume 1..1000; +0x128 next play time (1/10 s) |
| aux | +0x88 | play count (0 once, n, −1 loop, 9999 endless) |
| aux | +0x8c | volume 1..1000 (0 → 1000); DirectSound volume = (v−1000)·10 (1/100 dB) |
| aux | +0x90 | digits: units positional, tens restart-on-retrigger |
| aux | +0x94 | distance falloff divisor (attenuation = world units / falloff; 0 → 1) |
| aux | +0x98 | delay between plays in 1/10 s (<0: random first delay, see quirk) |
| aux | slot 0 | sound |

**Data check** (231 objects): 221 plain one-shots ([0,0,0,0,0], tag-started, k54 0, flags 0x01000230),
5 loops with count −1: LIPS01 #171 FIRE01 [−1,0,1] r=128 and #212 STEAM [−1,990,1] r=96 (tag −1,
no name → started only by the player walking into the large bump radius; positional), LIPS12 #213
FIRE01 [−1,500,1], LIPS13 #257 WATER08 [−1,1000,4] (tag 9999, bump-started, positional), LIPS15 #34
ALARM01 [−1,0,0] (tag 9, non-positional loop). No record uses delay/falloff/restart. All slot-0 names
resolve to Sound records except LIPS02 #7 (no aux) and LIPSTART #4 (slot 0 empty, slot 4 'AGONY' – a
leftover copy of a menu hotspot aux; silent). XRB +0x0c already contains the init bits (0x11 / 0x01).

---------------------------------------------------------------------------------------------

### OSNDSEQ (index 54, FUN_0042f615, mask 0x3002d0) – confidence H

Plays aux sound slots 0..4 one after the other (a chain through Sound+0x30), whole sequence
repeated per object +0x54. Re-triggering while it plays **stops** it (toggle).

**Events**
| event | action |
|---|---|
| 0x01 init | flags |= 0x200; +0x0c |= 2; with aux: resolve sounds; for each slot k with a Sound: Sound+0x28 (i16 per-sound play count) = aux int k + 1; +0x11c = 0. |
| 0x04 stop (direct) | if active and current sound still playing: stop the whole chain (FUN_0049d5aa from slot 0), inactive, +0x11c = 0. |
| 0x10 tick | see below. |
| 0x40 / 0x80 (edge) / 0x200 (FUN_0048a05a player-ish, edge) | if not active or current sound finished: start; else stop chain + inactive. |
| 0x100000 / 0x200000 | not handled. |

```
start: loops = self.+0x54; cur = slot0; link_chain(aux, loops)      // FUN_0049e482
       reps = cur.count - 1; play(src=other, cur, cur.count); active
tick (active, cur != null, finished(cur)):
    reps -= 1
    if reps >= 1 { play(cur, cur.count); return }      // same sound again
    cur = cur.next
    if cur == null {
        loops -= 1
        if loops < 1 && self.+0x54 != -1 && self.+0x54 != 9999 { inactive; return }
        cur = slot0
    }
    reps = cur.count - 1; play(cur, cur.count)
```
FUN_0049e482(aux, n): slot0→slot1→… linked via Sound+0x30, stopping at the first empty slot; if
n < 0 the last one links back to slot 0 (circular → endless). Plays of sound k = max(1, aux int k)
(count = int+1, `reps = count−1` is decremented before the replay test). The count is also passed
as the play parameter: count < 1 would mean hardware loop (only for int = −1, not shipped).

**Fields**: obj +0x54 sequence repeats (≤0 → once; −1/9999 endless), +0x118 loops left, +0x11c current
Sound, +0x120 remaining plays of it; aux slots 0..4 sounds, aux ints +0x88..+0x98 per-slot extra plays.

**Data check** (21 objects, LIPS02/09/12/14): k54 0 (16, once), −1 (4, endless ambience: DASTREET,
AMBWLK…, SUBBEG/SUBMID chains), 1 (1); aux ints 0/1 (one 2). All slot names resolve; each aux slot has
its own Sound record (duplicate names like SPEAKER3 get distinct records 637–641), so chaining does
not corrupt other users. 9 are tag −1 (bump-started by the player; r=16).

---------------------------------------------------------------------------------------------

### OTEXOVER (index 86, FUN_00436981, mask 0x3803d0) – confidence H (logic), M (overlay field meanings)

Toggles a full-screen/HUD picture overlay (an Overlay record, 72 B) with start/loop/stop sounds.

**Events**
| event | action |
|---|---|
| 0x01 init | flags |= 0x200; with aux: FUN_004367f4(self,0) builds the overlay; +0x1d3 |= 0x20 (marks "overlay object"; load scan FUN_0047f7a5 re-registers objects with this bit and +0x54 > 0 as DAT_0050ffe4). |
| 0x10 tick | cooldown +0x11c -= frame step (DAT_005670ec) while > 0. |
| 0x40 tag | trigger. |
| 0x80 / 0x200 (FUN_0048a05a player-ish) | edge test, trigger. |
| 0x80000 UI key | toggle immediately (no cooldown / mode check). |
| 0x100 | in mask, not handled. |
| 0x100000 OEFFOFF | if shown: stop slot 1, play slot 2; hide; overlay+4 |= 2 (locked). |
| 0x200000 OEFFON | clear lock; cooldown = 40.0; if +0x54 < 1: if hidden → play slot 0 once, slot 1 looped, show. Else (auto mode) re-evaluate FUN_004366d8(DAT_0050ffe4, player region, other). |

```
trigger: if cooldown > 0 || overlay == null return
         if self.+0x54 > 0 { DAT_0050ffe4 = self; return }      // auto mode: just register
toggle:  if hidden { if self.+4 & 0x80000000 return  // effect disabled
                     show; play(slot0, once); play(slot1, loop) }
         else      { hide; stop(slot1); play(slot2, once) }
         cooldown = 0x280000 (40 frame units ≈ 0.5 s)
```
Auto mode, FUN_004366d8(ovobj, region, src) – also called by the player region-change routine
FUN_004d8089 every time the player changes region: if hidden, not locked and region+0x34 (ceiling
texture) == null → show + slot0 once + slot1 loop; if shown and the region has a ceiling texture → hide
(+ stop slot1, play slot2 when not locked). I.e. an "outdoors only" overlay (rain). cooldown = 40.

FUN_004367f4(obj, p): allocate the Overlay (FUN_004d7b56) into obj+0x10c if missing. p == 0: overlay
+0x24..+0x34 = aux+0x88..+0x98; load picture named aux slot 9 into overlay TC (+0x0c) with
FUN_00488fae(tc, name, 0, 2) (start animation FUN_004895bb if tex+0x20 == −1, then tex+0x20 =
DAT_005675cc); if the pic loaded: +0x11c = +0x124 = 0, resolve sounds, and +0x54 > 0 → DAT_0050ffe4 =
obj. p ≠ 0: overlay pic (+0x14) = p, overlay flags |= 4 (raw-pic mode).

Overlay draw (FUN_0045675f, sorted by +0x30): only overlays with +4 bit 1; FUN_00456615 steps the
TC animation and blits pic at x = +0x28, y = +0x24 (centred when asked), with +0x2c passed as blit
mode (−1 = normal; menu items put a highlight value here), returning w/h into the caller's
pointers; if +0x34 ≠ 0 draws a rectangle outline in colour +0x34. Note the y/x order: aux+0x88 → +0x24
= **y**, aux+0x8c → +0x28 = **x** for OTEXOVER (OHOTSPOT/OMENU swap them, so there aux+0x88 = x).

**Fields**: obj +0x10c Overlay ptr, +0x11c cooldown (16.16 frame units), +0x54 >0 = auto
(outdoors) mode; overlay +4 bit 1 shown, bit 2 locked; aux ints → overlay +0x24 y, +0x28 x, +0x2c
blit mode, +0x30 z-order, +0x34 border colour; aux slot 9 picture; slots 0/1/2 = on / loop / off sounds.

**Data check**: one object, LIPS12 #116, tag 65, aux [0,0,15,0,0] slot 9 'RAIN2', no sounds, k54 0
(tag toggles a rain overlay at 0,0 with blit mode 15). Sibling effects OTEXOVUP (87, LAB_00436cfc) /
OTEXOVDN (88): force show (UP) on the overlay object named in their aux slot 9; unused in data.

---------------------------------------------------------------------------------------------

### OCURSOR (index 83, LAB_00441430, mask 0x1) – confidence H

Passive data object: the menu mouse cursor. Handler: event 1 → flags |= 0x200, resolve aux sounds;
a 0x200 branch calls FUN_00489f14 and discards the result (dead code; 0x200 is not in the mask).
All behaviour lives in OMENU:
* OMENU create (0x2000000) finds the object named in OMENU aux slot 8 (+0x6E, "MENUCURS"), stores it
  in the hotspot list (+0x18) and calls FUN_004367f4(cursor,0) → Overlay with picture = cursor aux
  slot 9 (HAND01/HAND03), position from aux+0x88/+0x8c (then x/y swapped).
* Mouse handler FUN_004416aa writes the scaled mouse position into the cursor overlay each frame
  (+0x28 = x, +0x24 = y).
* Menu draw FUN_00456aa4 draws it last, centred, blit mode −1 (no overlay-visible flag needed).
**Data**: MENUCURS in every level (aux [80,90,0,0,0], slot 9 HAND03, k54 0); LIPSTART uses HAND01,
k54 1 (unused).

### OHOTSPOT (index 84, LAB_00441adf, mask 0x1) – confidence H (init/draw/keys), M (mouse edge cases)

Menu item. The handler only handles event 1 (with aux): +0x1d0 |= 0x2000 (+0x1d1 bit 0x20; no reader
found); by kind (+0x54):
* 0 picture: FUN_004367f4 (pic = aux slot 9), swap overlay x/y (→ +0x28 = aux+0x88 x, +0x24 = aux+0x8c y),
  overlay+0x34 = aux+0x98 (border colour).
* 1 text, 4 save-slot: create Overlay if missing; +0x24 = aux+0x8c (y), +0x28 = aux+0x88 (x), +0x34 =
  aux+0x98, +0x3c (height) = 10, obj +0x11c = +0x124 = 0. No picture, **sounds not resolved**.
It never receives other events; selection and activation are done by OMENU code:

* Collection: OMENU on 0x2000000 allocates a 0x80-byte list (+8 count, +0xc selected index,
  +0x18 cursor object, +0x1c… item pointers; no bound check – ~25 max) and adds objects named
  `<prefix>1`, `<prefix>2`… (prefix = OMENU aux slot 9) until one is missing.
* Opening (OMENU on 0x40/0x80/0x200/0x80000): clears every item's +0x1d0 bits 0x04|0x10, selection
  = −1, plays OMENU slot 0, input mode DAT_00510204 = OMENU aux+0x88 if ≠ 0 (1 keys, 2 mouse, 3 both,
  default 1); OMENU +0x54 > 0 → mouse hit-testing on (DAT_00510214 = 1, FUN_00479c6c(hwnd,4)).
  Each 0x10 tick of the open menu runs the input handler(s); mode 3 runs keys first and the mouse only
  when the key handler returned −1 (no key this frame).
* Keys FUN_00441513 (last key DAT_00510da8): Up/Left: selection − 1 (from ≤0 wraps to last);
  Down/Right: + 1 (wraps to 0); Enter: close menu and **ActivateTag(item aux+0x94)** of
  list[selection] (quirk: with selection −1 the entry read is list+0x18 = the cursor object, so Enter
  before any selection fires the cursor's aux+0x94 – 0 for MENUCURS, i.e. ActivateTag(0)); Esc or the menu key
  DAT_00510dac: close without action. Closing = DAT_00510214 = 0, open-menu count −1,
  FUN_00479c6c(hwnd,1), OMENU +0x1d0 &= ~2.
* Mouse FUN_004416aa: client cursor pos scaled to 320×200 by DAT_005330c4/c8 (unless display mode
  DAT_0056e8e0+0x48 == 2). Hit box: x within the menu box (info x+2 … x+w−2) and y within (info y …
  y+h) – only when the menu info string was parsed; without it x uses the item (+0x28 … +0x28+width
  +0x38) and the y bounds are undefined (all shipped menus have info). Inside and within item y
  (+0x24 … +0x24+height +0x3c) → item +0x1d0 |= 4, selection = index; outside the box → item +0x1d0
  &= ~0x10 and selection = −1. Activation = mouse button **released** (DirectInput buttons
  DAT_005931dc+0x60..0x62 all up) after a press was latched (OMENU +0x1d0 bit 8, set whenever any
  button is down) while an item is selected → item +0x1d0 |= 0x10, close menu, ActivateTag(item
  aux+0x94), clear item bits, selection −1. Esc closes.
* Draw (FUN_00456aa4 via the menu renderer; colours selected DAT_00510728 = 176, normal DAT_00510724
  = 92, background = OMENU aux+0x98 (−1 none)):
  kind 0: blit the pic with mode = (u16 at item +0x48) when selected else −1; when selected also the
  namedmessage of aux slot 5 below it (y + pic height). Kind 1: namedmessage(aux slot 5) at (x,y);
  overlay width +0x38 = 8·len (used by hit-testing). Kind 4: FUN_004bbb86(save slot aux+0x90) →
  character name if that slot is in use, else behaves like kind 1 (shows the key text, e.g. `---`).
  Items whose key has no namedmessage are not drawn (kinds 1/4).

**Fields**
| rec | off | meaning |
|---|---|---|
| obj | +0x54 | item kind 0 picture / 1 text / 4 save slot |
| obj | +0x10c | Overlay (x +0x28, y +0x24, width +0x38, height +0x3c, border +0x34) |
| obj | +0x1d0 | bit 4 hovered, bit 0x10 clicked |
| aux | slot 5 (+0x4a) | namedmessage key = label |
| aux | slot 4 (+0x3e) | sound name – **never resolved or played** by any menu code found |
| aux | slot 9 | picture (kind 0) |
| aux | +0x88 / +0x8c | x / y |
| aux | +0x90 | save slot (kind 4); for kind 1 just the line index |
| aux | +0x94 | tag fired on activation |
| aux | +0x98 | border colour (kind 0) |

**Data check**: 88 objects. Level XRBs: MQHS1..4 (k54 1) x=64, y 40/64/80/96, tags 621 (save), 0
(continue = just close), 623, 629 (LIPS00 MQHS4 uses 699 instead of 629). LIPSTART: M1HS1 (601),
M1HS2..6 kind 4 slots 1..5 tag 602, M1HS7 629; M2HS1..5 tags 619/612/613/600/629 – matches script.md
§8.2. Slot-4 sound names (CLAGN01, AGONY, GRUNT01, ANVIL) have unresolved Sound refs (−1) in the XRB,
consistent with the handler not calling FUN_0049e0bd. No kind-0 hotspots shipped.

---------------------------------------------------------------------------------------------

#### [C] Shared helpers decoded here
| addr | meaning |
|---|---|
| FUN_0049fbe4 / FUN_0049fc30 / FUN_0049fc7c | game clock: seconds / tenths / 1/25 s (FUN_0049fa1c updates from GetTickCount) |
| FUN_0049e0bd(aux) | resolve aux sound names slots 0..4 into fresh Sound records (+0x9c..+0xac); once per aux (aux+4 bit 1) |
| FUN_004d7a8c / FUN_004d7af1 / FUN_004d7b56 | allocate Sound / Text / Overlay record |
| FUN_0049d6c7(snd, n) | play sound; n < 1 = loop; handles MIDI/CD type (flag 1) and object-proxy sounds |
| FUN_0049d933(src, snd, n) | play unless src is a 0x01000000-flag non-player beyond the sound range DAT_00567104 (when sound-range option DAT_0056e8e0+0x1c&1, set by the sound-settings effect handler FUN_00430f25 from its aux+0x98) |
| FUN_0049d489(snd) | stop sound (proxy Sound flag 0x400 → send event 4 to its object +0x38 via FUN_0049d411) |
| FUN_0049d411(obj) | send event 4 to an OSOUND (+0x0c&1) / OSNDGRP (&4) / OSNDSEQ (&2) object |
| FUN_0049d666(obj) | send 0x80 to such an object (script OSOUND(name) on an object) |
| FUN_0049da6b(snd) | 1 = finished / not playing (or sound off), 0 = playing |
| FUN_0049d5aa(snd) | stop every Sound in the +0x30 chain |
| FUN_0049e482(aux, n) | link aux sounds 0..4 into a chain, circular if n<0 |
| FUN_0049e2ad(snd, pan) / FUN_0049e307(snd, v) | set pan / volume ((v−1000)·10 hundredths of dB) |
| FUN_0042e007(obj) | pan from the angle in obj's point (+0x14→+0x34), ±10000·DAT_006c9e68 per quadrant (M) |
| FUN_0042e10a(listener, obj, x20, &dist) | octagonal distance; returns (dist>>16)/aux+0x94 (×20 if flag) |
| FUN_00468f3e(x1,y1,x2,y2) | approx distance dx+dy−min/2 |
| FUN_004d61ec | random (abs(rand+tick)) |
| FUN_00424862(name) | find Object by name (+0x58) |
| FUN_0043dcb6 / FUN_0043e252 / FUN_0043e11f / FUN_0043d707 / FUN_0043d631 | Text resource: load, next `[section]`, next line, find `[name]`, section size |
| FUN_0043d8d4(view, clear) | draw all OTEXTs |
| FUN_00475bb4(view,x,y,fg,bg,text,maxlen) | 8×8 text with optional bg box (bg ≥ 0) |
| FUN_004b149e(text, secs) | status-line message for ≥3 s |
| FUN_004367f4(obj,p) / FUN_004366d8 | build overlay from aux / outdoors auto-toggle |
| FUN_00456615 | draw one overlay (+anim step FUN_004564fd) |
| FUN_00441513 / FUN_004416aa | menu keyboard / mouse input |
| FUN_004d8089(oldReg,newReg) | player region change: 0x2000/0x1000 to regions & player, then overlay auto-toggle |


---

<!-- part D -->

Slot numbering: aux name slot k is at aux+0x0e+12k. So slot 5 = +0x4a, slot 6 = +0x56, slot 7 = +0x62,
slot 8 = +0x6e, slot 9 = +0x7a. int0..int4 = aux+0x88..+0x98. S0..S4 = Sound refs at aux+0x9c..+0xac.

Common "trigger gate" used by all six handlers (call it TRIGGER_GATE):
- **0x40 (tag)**: goes straight to the action. No contact test.
- **0x80 (trigger)**: action only if not already in contact, i.e. (self+4 & 0x0f) == 0. This is the
  edge test from the brief.
- **0x200 (bump)**: `other` must be non-null. It must pass an activator check (which one is listed per
  effect). Then the same edge test applies.

New fact about ActivateTag: `FUN_00472b7c` takes a second register argument, EDX = activator. It passes
the activator on as the "other" object (handler EBX) of the 0x40 event. OTAGSW passes the bumping
object. The handlers in this section call ActivateTag without setting EDX, so the activator is
whatever stale value EDX holds. A reimplementation should pass the original activator, or null.

---

### OPICKUP (index 130, FUN_0045dc0e, mask 0x2d0) — confidence H for single player, M for the CTF branch

**Trigger events**
- **1 (init, direct call)**
  - Clear +0x118 and +0x124.
  - Home point:
    - If the object has no home point (+0x18 == 0), allocate a new Point (`FUN_004d6d98`, AddPoint)
      and copy the current position Point (+0x14) into it, keeping the new point's index.
    - Store the angle in the point: point+0x34 = (u16 angle +0x24) << 16.
  - Snapshot point:
    - Allocate the +0x1bc Point if it is missing.
    - Always re-copy the current position into it.
  - Resolve the aux sounds (`FUN_0049e0bd`).
  - If (+0x54 & 1), set +0x0c |= 0x08000000.
  - Flag changes:
    - set +0x10 |= 0x20000000;
    - clear +4 bit 0x1000 (un-hide);
    - clear flags 0x200 (movable);
    - set +0x1d0 |= 0x80000000.
- **0x10 (tick)**: only for "team flag" items (+0x0c & 0x800). See the CTF note below. Normal pickups
  ignore it.
- **0x40 / 0x80 / 0x200**: TRIGGER_GATE. On 0x200:
  - return if the item is hidden (+4 & 0x1000, i.e. already carried);
  - activator check: aux int4 > 0 → `FUN_0042426a(self, other, int4)`, else `FUN_00489ff8(other)`
    (player-ish).

**Fields**

| record | off | type | meaning |
|---|---|---|---|
| obj | +0x18 | Point* | home point (created at init); used by ODROP mode 2 and the CTF return |
| obj | +0x1bc | Point* | position snapshot refreshed at every init |
| obj | +0x0c | u32 | bit 0x800 = team-flag item (CTF); 0x1000 = follow carrier; 0x2000 = timed return; 0x08000000 is set from +0x54 bit 0 |
| obj | +0x118 | i32 | CTF message cooldown (0x500000 = 80 time units) |
| obj | +0x124 | u32 | CTF return time; pickup is blocked while > 0 unless +0x0c & 0x02000000 |
| obj | +0x194 | Object* | CTF: carrier / base object |
| obj | +0x1ae | i16 | team (compared with the picker's team) |
| aux | int0 | i32 | **tag fired on pickup** (if > 0) |
| aux | int1 | i32 | tool index 1..15 granted by the pickup (0 = none) |
| aux | int4 | i32 | activator filter code; also "auto-select the granted tool" when > 0 |
| aux | slot 8 / 9 / 7 | name | objects registered as tool / projectile / explosion for tool int1 |
| aux | S4 | Sound* | pickup sound |
| picker ctrl (+0x1b8) | +0x264 | Object*[64] | **inventory**: pointers to the item objects |
| picker ctrl | +0x364 | i32 | inventory count (at most 64) |

**Behaviour (normal item, +0x0c & 0x800 == 0)**
```
on_pickup(self, other):
  if self.+0x124 >= 1 and !(self.+0x0c & 0x02000000): return       // (only ever set for CTF items)
  picker = player_of(other)              // FUN_0048a05a: the player, or the owner of a player projectile
  if picker: inventory_add(picker, self)  // hides the item (+4|=0x1000) and makes it immovable (flags|=0x200)
  net_host_update(self)                   // FUN_0048993d, multiplayer sync queue only
  wake(self)
  if aux:
    t = aux.int1
    if 0 < t < 16:                        // weapon pickup: register the tool objects
      for (slot, kind) in [(8, TOOL=1), (9, PROJ=2), (7, XPLD=5)]:
        o = find_object(aux.slot); if o: o.+0x54 = t; register_tool(o, kind)   // FUN_004657a3
      if other == null or !(other.flags & 0x40000): return   // NOTE: no tag and no sound in this case
      if aux.int4 > 0 and tool[t] and proj[t]: select_tool(other, t, force=0)
    if aux.int0 > 0: ActivateTag(aux.int0)
    play_sound_at(other, aux.S4)          // FUN_0049d933
```
- Inventory items are **object pointers**, not types or counts. There is no stacking: each item
  object takes one slot, and adding the same object twice is ignored. A full inventory (64)
  silently refuses the add. The item then stays visible and fires its tag and sound again on every
  new contact.
- The item is not deleted. It stays in the world, hidden (+4 bit 0x1000) and skipped by bump while
  carried. ODROP / DropOnePossession bring it back.
- With no picker (for example a tag or trigger sent by a non-player), nothing is hidden. Only the
  tag and the sound happen.
- There is no respawn in single player.
- One-shot in practice, because the item is hidden after pickup.

**CTF / team branch (+0x0c & 0x800; not used by Lips)**
- On contact by a picker of the **same team** (+0x1ae): show "own flag" message (team text +0x3a4) to
  the local player, rate-limited by +0x118.
- On contact by a picker with **no team**: show a message from DAT_0056e69c+0x360.
- On contact by any **other team**: `FUN_0041230b(other, self)`, which takes the flag.
- The flag is not taken while (+0x0c & 0x1000) == 0 and other.+0x0c & 0x40.
- Tick (0x10):
  - if +0x0c & 0x2000 or +0x0c & 0x02000000: when `time() >= +0x124`, call
    `FUN_00413062(+0x194, self)` to return the flag;
  - else if +0x0c & 0x1000 and a carrier is set: move to the carrier's position, at z + carrier
    height (`FUN_0042ccbc`), and wake.

**Data check (251 objects)**
- Flags:
  - all have +0x10 = 0x20000000 and +0x1d0 = 0x80000000 in the file as well. Init re-asserts both;
  - all are type 0x0100002x/3x (0x20 = bump-collidable);
  - 41 also have flags 0x2;
  - tag = -1 everywhere, so 0x40 never reaches a pickup;
  - one item (LIPS01 #211) has k54 = 1 and +0x0c = 0x08000000.
- Aux:
  - 155 have **no aux**. These are mostly unnamed loot: inventory only, no tag, no sound;
  - 96 have aux ints = [T, 0, 0, 0, 0] with T in 23..144. So int1..int4 are always 0: no weapon
    pickups and no filter.
- **int0 is a tag, not a value.**
  - Receivers of those tags (objects with +0x44 == T): OTEXT 89, OTIMER 18, OWTOW 9, OSETF 4,
    OOTOO 4, OTAGSW 2, OSOUND 2, OEFFSW 2, OVIEWON 1, OINCOS 1.
  - So the usual pattern is: pick up an item, its OTEXT message object shows "you found ...", and a
    door, lift or timer may run.
  - 11 tags have no object listener. They are probably wall or region tags, or unused.
- S4 = pickup sound (POP01, CLICK01, KLANG01, ...).

---

### ODROP (index 131, handler FUN_0045e4b2, mask 0x2c0) — confidence H

"Use an inventory item here." It takes the named item out of the player's inventory and places it in
the world, for example a key put into a lock.

**Events**
- **1**: flags |= 0x200 (immovable). Resolve sounds.
- **0x40 / 0x80 / 0x200**: TRIGGER_GATE. The 0x200 activator check is aux int4 > 0 ? filter(int4) :
  player-ish(other).

**Fields**

| record | off | meaning |
|---|---|---|
| aux | slot 9 (+0x7a) | name of the item object to take |
| aux | slot 8 (+0x6e) | name of the **Point** to drop it at (`FUN_00424911`, find-point-by-name) |
| aux | int0 | tag fired after a successful drop |
| aux | int1 | if 1..15: clear tool / projectile / explosion slot int1 and reset the player to tool 0 |
| aux | int4 | activator filter code |
| aux | S4 | success sound |
| item aux | int1 | tool index. If it equals the current tool, the tool is deselected and made unavailable |
| item aux | int2 | tag fired by the item when it is dropped |
| item aux | int3 | drop mode: 0 = put at the point; 1 = destroy (+0x20 = 0x10000000); 2 = return to the home point +0x18 |
| item aux | S3 | drop sound |

**Behaviour**
```
on_trigger(self, other):
  require self.aux and current_view.camera_obj(+0x2c) != null and aux.slot9 != ""
  pt = find_point(aux.slot8)                       // may be null
  for obj in all_objects where obj.name == aux.slot9:
    picker = player_of(other)
    if picker: r = inventory_remove(picker, obj)   // 1 if it was carried (swap-with-last removal), else -1
    if r != 1: continue           // the original reads an uninitialised r if picker is null
    obj.+4 &= ~0x3000                              // un-hide, re-activate
    dest = pt ?: (obj not CTF ? copy_of(self.position) : self.position)
    drop_one_possession(other, obj, dest, mode=0)  // FUN_0045e1f4
    if aux.int0 > 0: ActivateTag(aux.int0)
    if 0 < aux.int1 < 16:
      tool[int1] = proj[int1] = xpld[int1] = null
      if other.flags & 0x40000: select_tool(other, 0, 0)
      return                                       // no sound and no wake on this path
    play_sound_at(other, aux.S4); wake(obj); return  // only the first carried match is used
```
- If the player does not carry the item, nothing happens. The effect does not fire a "fail" tag.
- It can repeat. Each trigger consumes one matching carried item. Because items are matched by
  name, several items with the same name are handled.

**Data check (43 objects)**
- All have flags 0x01000230 and a tag (22..121), so they are tag-driven. They are usually fired by an
  OTAGSW or OBUMPSW lock switch.
- slot 9 always names an existing pickup object: keys, gems, BRONZE/SILVER/GOLD and so on.
- slot 8:
  - set in 20 of 43;
  - names an existing Point in all of them except "KEYRET" (LIPS03) and "BRNSBAK" (LIPS06). For
    those two the item falls back to the ODROP object's own position.
- int0 = follow-up tag in 41 of 43. Its listeners: OTEXT 24, OWTOW 12, OSETF 9, OSETC 1, OINCOS 1.
  This is the "door opens" logic.
- int1..int4 = 0 everywhere. Item aux int2 and int3 are always 0, since pickups only use int0. So in
  Lips the item is simply placed at the point (or the ODROP position) and becomes visible again
  (often a key shown in a lock).

---

### OSELTOOL (index 127, handler FUN_00464ae9, mask 0x2c0) — confidence H

**Events**
- **1**: flags |= 0x200. Resolve sounds.
- **0x40**: always acts.
- **0x80**: acts if not in contact.
- **0x200**: acts if `FUN_00489ff8(other)` (player-ish) is true and the object is not in contact.

**Fields**
- obj +0x54: tool index 0..15.
- aux int1:
  - 0 or no aux: plain select;
  - < 0: make the tool unavailable, then force-select it;
  - > 0: make it available, then force-select it.
- aux int0: tag fired afterwards.

**Behaviour**
```
idx = self.+0x54
if 0 <= idx < 16:
  if !aux or aux.int1 == 0: select_tool(PLAYER, idx, force=0)      // only if bit idx of AVAILTOOLS mask set
  else: if int1 < 0: avail &= ~(1<<idx); if int1 > 0: avail |= 1<<idx
        select_tool(PLAYER, idx, force=1)
if aux and aux.int0 > 0: ActivateTag(aux.int0)
```
- It always acts on the local player (DAT_00510778), not on `other`.
- The availability mask is DAT_00543cfc, the AVAILTOOLS bitmask. The XRB header +0x50 initialises it
  to 1.

**Data check**
- Only 2 objects: LIPS00 #687 and LIPS17 #243.
- Both are unnamed, have no aux, k54 = 5 and tag 9999. ActivateTag can never fire 9999 from scripts,
  and the objects have no name for a 0x80 trigger.
- So they work by **walking into them**: they select tool 5 if it is available.

---

### OINCOS (index 112, handler FUN_0044be5d, mask 0x2d0) — confidence H

**Events**
- **1**: flags |= 0x200, +0x10 |= 0x20000000, resolve sounds.
- **0x10**: in the mask but ignored.
- **0x40 / 0x80**: target defaults to the player.
- **0x200**: target = `FUN_0048a05a(other)`, which must be non-null. The edge test applies.

**Fields**

| record | off | meaning |
|---|---|---|
| aux | slot 9 | optional target object name. If found, it overrides the default target |
| aux | int0..int4 | deltas for Life, Magic, Armor, Money, Experience (states 1..5) |
| aux | S4 | sound (non-positional `FUN_0049d6c7`) |
| target | +0x1b4 | state block. States are at +0x198 + 4*i. Nothing happens without it |

**Behaviour**
```
target = find_object(aux.slot9) ?: (event == 0x200 ? player_of(other) : PLAYER)
if target.+0x1b4:
  for i in 1..5: change_state(target, i, aux.int[i-1])      // FUN_0044bde9
  play_sound(aux.S4)
```
- `FUN_0044bde9(obj, statebuf, i, delta)` picks the update rule:
  - the player goes through the threshold logic `FUN_0044b9ee` (script.md §7: clamp to min/max and
    fire min/low/high/max tags on crossing);
  - the opponent uses `FUN_0044bbcf` (its own tables);
  - any other object uses `FUN_0044bdb0`, which is presumably a raw add.
- A delta of 0 has no effect.
- It repeats on every trigger. There is no one-shot logic.

**Data check (4 objects, all tag-driven, flags 0x01000230)**

| level / object | tag | ints | effect |
|---|---|---|---|
| LIPS10 #106 | 11 | [0,0,0,0,100] | +100 Experience |
| LIPS14 #74 | 81 | [0,0,0,0,100] | +100 Experience |
| LIPS14 #48 | 79 | [0,0,0,-1,0] | −1 Money |
| LIPS14 #105 | 30 | [100,100,100,100,500] | full refill + 500 Experience |

- No slot 9 is set, so the target is always the player.

---

### OSETF (index 41, handler FUN_0043c6de, mask 0x2d0, 68 uses) and OSETC (FUN_0043cb39, mask 0x2d0, 4 uses) — confidence H (M for the crush details)

These are **not OS-state commands**. OSETF means *set floor* and OSETC means *set ceiling*. Each one
moves the floor (region +0x0c) or the ceiling (region +0x10) of a named region to an absolute height
at a given speed: a lift, platform, pit or crusher. They can also swap the floor or ceiling texture.
The two handlers are the same code. Only the region field, the busy bit and the helpers differ:

| | floor field | busy bit | start helper | step helper |
|---|---|---|---|---|
| OSETF | region +0x0c | region +4 bit 0x2000 | `FUN_0043c4c0` | `FUN_0043bc92(self, 1)` |
| OSETC | region +0x10 | region +4 bit 0x4000 | `FUN_0043c911` | `FUN_0043bc92(self, 2)` |

**Fields**

| record | off | meaning |
|---|---|---|
| aux | slot 9 | region name (`FUN_0044e0ba`, find-region-by-name: region +0xa4) |
| aux | slot 8 | optional new floor / ceiling texture name (`FUN_00488fae` on region TC +0x1c or +0x34) |
| aux | int0 | target height, absolute world units → obj +0x178 (16.16) |
| aux | int1 | speed → obj +0x17c (16.16). ≤ 0 = no motion (texture only) |
| aux | int2 | → obj +0x180. The value −9999 means "silent mode": no loop and no stop sounds |
| aux | S0 / S1 / S2 | start / moving loop / stop sounds. They play only if the player is in the region or within DAT_00567104 of its centre (`FUN_004d96c4`) |
| obj | +0x174 | Region* found at init |
| obj | +0x184 | current target height |
| obj | +0x120 | signed per-frame step (speed × frame time via `FUN_00473195`) |
| obj | +0x124 | 1 = moving |
| obj | +0x11c | texture-applied / sound-state flag |
| obj | +0x0c bit 0x400 (byte +0xd & 4) | "pending": the trigger was queued because the region was busy |
| region | +0xf4 | the OSETx object currently driving the region. Its loop sound is stopped when another one takes over |

**Events**
- **1**:
  - flags |= 0x200;
  - `FUN_0043b2b3`: find the region and load int0, int1, int2 as 16.16;
  - resolve sounds;
  - +0x124 = 0.
- **0x40 / 0x80 / 0x200**:
  - TRIGGER_GATE, where 0x200 needs `player_of(other)` and 0x80 and 0x200 need no contact;
  - also requires: region found, not moving (+0x124 != 1), aux present, not pending.
- **0x10**: advance the motion, and start a queued move.

**Behaviour**
```
start(self):                                   // FUN_0043c4c0 / FUN_0043c911
  compute(self):                               // FUN_0043b59d(self, which, 1)
    if speed <= 0 or int2 == -9999: just apply the slot-8 texture (once); step unchanged
    else: stop previous driver's loop sound; region.+0xf4 = self
          target = int0<<16 (absolute; mode 2 = relative exists but is unused here)
          step = scale(speed) * sign(target - cur), 0 if already there; apply slot-8 texture
  if step == 0: play S2 (stop) ; done (+0x124 stays 0)
  else if speed <= 0: instant set height = int0 (FUN_0043b99c)      // unreachable in practice
  else: +0x124 = 1; [OSETC: region busy bit set here]; step once
        stop S1, play S0 (start), play S1 looping

on_trigger: if region.busy: self.pending = true
            else: start(); if moving: region.busy = true else region.busy = false; pending = false
tick:       if !pending: if moving: step(); if !moving: region.busy = false; pending = false
            elif !region.busy: start(); (if it ended at once: clear busy); region.busy = true; pending = false
step(self):                                    // FUN_0043bc92
  h += step; clamp at target: on reaching it h = target, +0x124 = 0, stop S1, play S2
  carry_objects(region, step, which)           // FUN_0043b380
```
`carry_objects` (`FUN_0043b380`) runs over the objects in region list (+0xb4, next +0x14c):
- When the floor goes down, objects with +0x40 > 10.0 and not flags&2 are lowered with it: z (+0x2c)
  and rest z (+0x30) both get the step.
- Any object below the floor is raised to it.
- Any object whose head (z + height +0x34) reaches the ceiling is pushed down to ceil − h − 2.
- If the object is then below the floor (crushed):
  - for the player (flags 0x40000) the motion is stopped against them. OSETF sets
    floor = ceil − h; OSETC sets ceil = floor + h;
  - other objects just clip.
- Each move also syncs the object's point z and, for index 0, calls `FUN_004d848b`.

Notes:
- The busy bit is shared by every OSETF on the same region, so for example an ELEV02UP trigger
  pressed during ELEV02DN is queued.
- Quirk: in the tick path, the busy bit is set after a queued start even if the move ended at once
  (step 0). This can leave the region locked. A clean reimplementation should clear it in that case.
- Quirk: `FUN_0043b99c` in ceiling mode reads an uninitialised "old floor". That path is unreachable
  anyway.
- The moves can repeat. Each trigger moves to the same absolute height, so for a two-way lift the
  level uses two OSETF objects (UP and DN) on the same region.

**Data check**
- OSETF: 68 objects; OSETC: 4 (LIPS00 #21/#27 ELEV02, #281 DARKNESS, LIPS11 #26 DOM01).
- All have flags 0x01000230. Most are tag-driven (tags 21..134). 9 have tag −1 and are driven by bump
  or a named 0x80.
- slot 9 names an existing region in **72 of 72**.
- slot 8, where set (18), is always an existing Tex name (FLUIDRD, FLUIDGN, WOOD7V, SUBWY02H, ...),
  for example a pit filled with fluid.
- int0:
  - −2176..1920, a world height;
  - for example ELEV02UP −384 and ELEV02DN 0 on the same region; BARnDOWN −176.
- int1 speed is 20, 50, 100, 200 or 500. int2..int4 are 0 everywhere, so −9999 silent mode is
  unused.
- Sounds: S0/S1/S2 = ELEV02A/B/C (start / loop / stop) on lifts, or FWOOMP / AMBSFX on traps.

---

#### [D] Sub-functions decoded

| address | meaning |
|---|---|
| FUN_0045b69c(owner, item) | **inventory add**. The list is owner ctrl(+0x1b8)+0x264, Object*[64], count +0x364. Duplicates are ignored. Sets item +4 \|= 0x1000 (hidden) and flags \|= 0x200 |
| FUN_0045b744(owner, item) | **inventory remove**, swapping the last entry in. Returns 1 if the item was found, else −1 |
| FUN_0045e1f4(owner, item, point, mode) | **DropOnePossession**: remove from the inventory; place the item by its aux int3 (1 destroy, 2 home point +0x18, else `point`); if it was the player's current tool (item aux int1 == ctrl+0x20), deselect it; set DAT_00515528 = 1 (inventory changed); clear item +4 bits 0x3000 and flags 0x200; CTF return handling (mode 2/3/4); fire item aux int2; play item S3; wake; set item +4 \|= 0x0f so it is not re-picked while touching |
| FUN_0045e439(owner, point, mode) | drop all possessions |
| FUN_0045b182(owner, obj) | attach obj to owner (8 slots at ctrl+0x368, stride 0x10, with xyz offsets); sets obj +4 \|= 0x00100000\|0x1000 and flags 0x200. Probably carried or attached objects |
| FUN_00424862(name) | find an **object** by name (obj +0x58). Empty name → null |
| FUN_00424911(name) | find a **Point** by name (point +0x44) |
| FUN_0044e0ba(name) | find a **Region** by name (region +0xa4) |
| FUN_004d6d98(x) | AddPoint: allocate a new runtime Point (pool DAT_00567728, count DAT_0056772c) |
| FUN_0042ccbc(obj, point) | move an object to a point's position (used by drops and by the CTF follow) |
| FUN_004657a3(obj, kind) | register obj as tool slot obj.+0x54 (negative = not available). Kind: 1 player tool → DAT_0053fbe4[16] (also sets the AVAILTOOLS bit); 2 player projectile → DAT_00543024; 3 other tool → DAT_0053fc64; 4 other projectile → DAT_00543ca4; 5 player explosion → DAT_00542fa4; 6 other explosion → DAT_005422a4. Aux int1 /1000 % 1000 = team, giving per-team variants; obj +0x1ae = team |
| FUN_0047b766(player, idx, force) | select tool idx. Requires the AVAILTOOLS bit (DAT_00543cfc) unless force. Fires tag 920 + old index on a change; sets ctrl +0x28/+0x2c/+0x30 = tool / projectile / explosion objects (team variants in team mode) and ctrl +0x20 = idx; fires tag 900 + idx; writes state +0x156 = idx |
| FUN_0047b67a(player, clear) | deselect: optionally clear the availability bit, fire 920 + current, revert to tool 0, fire tag 900 |
| FUN_0044bde9(obj, statebuf, i, delta) | change a state: player → `FUN_0044b9ee` (thresholds), opponent → `FUN_0044bbcf`, else `FUN_0044bdb0` |
| FUN_0049e0bd(aux) | resolve aux sound names slots 0..4 (not "NO_NAME") into Sound refs +0x9c..+0xac, once (aux+4 bit 1) |
| FUN_0049d6c7(sound, once) | play a sound. once ≤ 0 = loop. Handles MIDI streams |
| FUN_0049d933(obj, sound, once) | play a sound for an object. In network modes it is distance-gated (DAT_00567104) against the local player; otherwise the same as `FUN_0049d6c7` |
| FUN_0049d489(sound) | stop a sound |
| FUN_004d96c4(obj, region, dist) | true if obj is in the region or within dist of the region centre (+0xd4/+0xd8) |
| FUN_00473195(v16) | scale a 16.16 per-time-unit speed by the frame time (table DAT_0056e470 for < 1024, else × DAT_005670e8) |
| FUN_0043b2b3 / 0043b59d / 0043c4c0 / 0043c911 / 0043bc92 / 0043b99c / 0043b380 | OSETF/OSETC: init / compute target / start floor / start ceiling / step / instant set / carry and crush objects (described above) |
| FUN_00488fae(TC*, name, 0, mode) | set a texture by name, loading it if needed |
| FUN_0048993d(obj, 0) | queue a network host update (≤ 120 per frame); no-op in single player |
| FUN_0041230b, FUN_00413062, FUN_00412752, FUN_00412b59 | CTF take / return / timed return / place flag (not decoded) |
| FUN_00472b7c(tag, EDX=activator) | ActivateTag. The second register argument is forwarded as the "other" object of event 0x40 |

#### [D] Open questions
1. Object +0x1d0 bit 0x80000000 (set by OPICKUP init) has no byte-test reader in the binary. It may
   be read with a sign test, or by the GAM inventory save. Object +0x10 bit 0x20000000 is tested in
   `FUN_00420817` (overlay or map object drawing: such objects are drawn in mode 2), so it is probably
   "show as an item marker". L.
2. Pickups and drops call ActivateTag with a stale EDX as the activator. Which value really arrives
   is not traced.
3. Treasure counter state 10 ("TF"): OPICKUP does not increment it. It must come from a script
   (STATEVAL) or another place.
4. The meaning of +0x0c 0x08000000 (set from +0x54 bit 0; one item in LIPS01) is unknown.


---

<!-- part E -->

Common notation: `self` = the object running the effect, `ev` = event (EDX), `other` = EBX extra.
`aux` = self+0x1b0 (may be NULL). `s9` = aux name slot 9 (aux+0x7a), `s8` = aux+0x6e, `s6` = aux+0x4a.
`S4` = aux+0xac (Sound resolved from name slot 4). `int0..int4` = aux+0x88..+0x98.

**EDGE(self)**: the handler returns when any of self+4 bits 1|2|4|8 is set. That means self is already in
contact (this frame or the last one), so a bump fires once per contact.

**TRIGGER(name)**: `t = find_by_name(name)` (FUN_00424668). If found, dispatch(t.effect, t, 0x80) (FUN_00472a54).
The normal dispatch filter applies, so a target disabled by OEFFOFF or without 0x80 in its mask ignores it.

**Clock**: `now()` = FUN_0049fc30. It returns a global counter in **1/10 s of real time (GetTickCount)**. It is
**not** the per-frame 16.16 unit in DAT_005670ec. All OTIMER/ORANDOM periods below are in tenths of a second.

---

### OTAGSW (index 20, FUN_00429090, mask 0x280). Confidence H

Events
- 0x01 init: set flags 0x200 (self+1 |= 2, *immovable*). Set self+0x1d3 |= 1 (*active collider / shootable*, see below).
- 0x200 bump: apply EDGE. Then the activator check: if `aux && int4 > 0`, use the filter FUN_0042426a(self, other, int4).
  Otherwise accept if FUN_0048a05a(other) != 0, i.e. other is the player, a player-ish object, or a
  projectile owned by one. A player's shot therefore triggers it. On a fail, return.
- 0x80 trigger: no checks.
- Anything else: ignored.

Fire (after 0x200 passes, or on 0x80):
```
if flags & 0x1000 == 0 {                    // unlimited
    if aux { TRIGGER(s9) }
    ActivateTag(self.tag)                   // int(self+0x44)
} else if self.k54 > 0 {                    // limited uses
    if aux { TRIGGER(s9) }
    ActivateTag(self.tag)
    self.k54 -= 1
}                                           // k54 == 0 -> dead switch, object stays
```
Fields

| rec | off | type | meaning |
|---|---|---|---|
| obj | +0x00 bit 0x1000 (+1&0x10) | flag | limited-use switch |
| obj | +0x00 bit 0x200 (+1&2) | flag | set at init: immovable (a bumping mover is put back, not pushed) |
| obj | +0x1d3 bit0 | flag | set at init. (a) The per-frame loop FUN_004a0375 runs the object-collision pass FUN_004a4a1f on objects with this bit (or the wake bit +7&0x10), even when they are not moving, so the switch actively detects contact. (b) OPROJ (FUN_0045f421, 0x200 branch) normally ignores collisions with immovable (flags 0x200) objects. It still collides when the other object has this bit. Together: *projectiles hit tag switches*. |
| obj | +0x44 | 16.16 | tag that is fired (int part) |
| obj | +0x54 | i32 | remaining uses when flags 0x1000 |
| aux | +0x7a | char[12] | slot 9: name of object/wall/region/... that gets 0x80 |
| aux | +0x98 | i32 | activator filter code (0/neg = default "player or player's projectile") |

Edge cases: repeatable per contact when unlimited. Nothing is hidden or destroyed. The switch does not
listen to 0x40, so firing its own tag cannot loop. A NULL aux is fine: only the tag fires.

Data check (93 objects): 88 have no aux and only fire their tag. All filter codes are 0. 82 are unlimited.
11 have flags 0x1000 with k54 = 1 (7 objects, one-shot), 4, 14, 20 or 30. Two have s9: LIPS00 'GRAVEYRD'
and 'YAYLAMP', both OTEXT, both present. Tags are heard by OTAGSW (many switches share one tag), walls,
regions, OSOUND, OTHING, OTEXT, OPICKUP, WTAGSW, RTDOOR and others. Exceptions:
- LIPS12 #18 tag 54 has no other listener and no ONTAG[54] (a dead switch, or used by script).
- LIPS06 #226 has tag -1, flags 0x1000202 and no aux, so it is inert.

Flags are usually 0x01000230. The 0x20 bit is the collision participant flag.

### OBUMPSW (index 22, FUN_004295bc, mask 0x280). Confidence H
Same as OTAGSW with two differences:
1. Init sets only flags 0x200. It does **not** set +0x1d3 bit0, so it is not tested every frame and not shootable.
2. The default activator test (int4 <= 0) is FUN_00489ff8(other). That test passes only for the **direct**
   player-ish body: flags 0x40000, or flags 0x80, or +0x10 bit 0x200000. Projectiles never pass.

So this is a "touch with your body" switch: TRIGGER(s9) and ActivateTag(own tag), with the same limited-use logic on +0x54.

Data check (162): none have flags 0x1000, 160 have no aux, and no s9 or filter is set anywhere. Every tag
has listeners: mostly other OBUMPSW sharing the tag, plus OSOUND (227 links), OFLYOFF/OFLYON and OTELE.
This fits bump-to-play-sound props and bump-to-toggle-flying.

### OOTAGSW (index 21, FUN_00428c95, mask 0x280). Confidence H (0 uses in shipped levels)
Init is like OTAGSW (0x200 and 0x1d3 bit0). On 0x200: EDGE, then the filter runs **only if int4 > 0**.
With no code, *any* object that bumps it triggers it ("OO" = object-operated). Fire: TRIGGER(s9).
Then if `aux && int0 > self.tag`, activate every tag in self.tag..=int0 (FUN_00472ac1). Otherwise
ActivateTag(self.tag). The limited-use logic matches OTAGSW.

### OOBUMPSW (index 23, FUN_00428ee2, mask 0x280). Confidence H (0 uses)
Same as OOTAGSW: it also sets 0x1d3 bit0, and the filter runs only if code > 0. The difference is no tag
range: it fires only ActivateTag(self.tag).

### ORANTAGS (index 25, FUN_004293e8, mask 0x280). Confidence H (0 uses)
Like OBUMPSW for init (0x200 only). Its default activator is FUN_0048a05a (player or player's shot), as in
OTAGSW. On fire it runs TRIGGER(s9), then `t = pick_random_tag(self)` (FUN_00429254); if t > 0, ActivateTag(t).
The limited-use logic is the same.

pick_random_tag:
- If slot-8 text starts with 'R': uniform in [int0, int1] (FUN_00423b0b). When int1 - int0 < 1 the result is 0 (none).
- Otherwise: a uniform choice among the positive values of int0..int4.
- Note: int4 is also the activator-filter code, so a filter code would also be a candidate tag.

---

### OTIMER (index 80, FUN_0042c5ff, mask 0x3002d1). Confidence H
A one-shot delay. When started, it waits `k54` tenths of a second, then runs TRIGGER(s9) and plays S4.
It does **not** fire its own tag and does **not** repeat on its own.

Runtime fields: self+0x118 = deadline (u32, clock units), self+0x11c = mode (copy of aux int0),
self+5 bit 0x80 (= +4 bit 0x8000) = *running*.

Events
- 0x01 init (needs aux): deadline = k54, mode = int0, flags |= 0x200, resolve aux sounds (FUN_0049e0bd).
  The running bit is not set, so a timer is stopped at level start.
- 0x40 tag: start, with no edge test.
- 0x80 trigger: EDGE, then start.
- 0x200 bump: requires FUN_0048a05a(other) (player or player's shot), then EDGE, then start.
- start: `if !running || mode != 1 { deadline = now() + k54; running = true }`. Mode 0, the only mode in
  the data, is retriggerable: a new start pushes the deadline back. Mode 1 ignores starts while running.
- 0x10 tick (needs aux): `if running && now() > deadline`:
  - find s9; if it is missing, return and keep running (retry every tick);
  - otherwise send it 0x80, play S4 non-positionally (the tick's EBX is 0), and clear running.
- 0x100000 (sent by OEFFOFF): clear running (stop, nothing fires). Also clear self+7 bit 0x80, the
  dispatcher's disable bit, so the timer stays enabled.
  OEFFOFF (FUN_00433547) sends 0x100000 first. It then sets the disable bit only when the target's effect
  name is **not** "OTIMER". So OEFFOFF on a timer means "cancel", and the timer can be restarted later.
- 0x200000 (sent by OEFFON, because the mask contains 0x200000): falls through and is ignored. OEFFON only
  clears the disable bit, so it does **not** start a timer.

Fields: obj +0x54 = period (1/10 s), +0x44 = start tag, aux s9 = target, aux int0 = 1 for no retrigger,
aux S4 = sound on expiry. Slot-0 sounds are resolved but not used here.

Data check (46, all with aux):
- int0 = 0 everywhere. k54 is 0–130, i.e. 0–13 s. Common values: 30 (3 s), 50 (5 s), 120 (12 s).
- k54 = 0 (THUNDER1) fires on the next clock step.
- Every s9 resolves in its level. Targets: OSETF (a lift returning after 3–5 s), OTEXT, OSOUND, OTAGSW
  (which chains to that switch's tag), ODESTROY, OTELE, OWTOW, OOTOO, OVIEWOFF, OFLYOFF, OSNDSEQ.
- One tag can start several timers with staggered periods, e.g. LIPS02 tag 52 starts GAS01–GAS05 at 2/3/4/6/9 s.
- Tag 9999 (LIPS09) starts at level load.
- Two timers have tag -1 and are started only by 0x80. LIPS14 STRTDONE is started by OCOUNT COUNTER.
- Two use S4 (FIZZ, BUZZER01).
- Units: a 5 s lift-return delay is plausible; 5 s in frame units would be about 50 s, which is not.

### ORANDOM (index 81, FUN_0042c805, mask 0x2d1). Confidence H
A random-interval repeater, or a one-shot random delay.

Runtime fields: +0x118 deadline, +0x11c mode = int0, +0x120 shots left (= int1 if > 0, else -1 = unlimited),
+5 bit 0x80 running.

delay():
```
if int2 >= 1 && int3 > int2 { int2 + rand % (int3 - int2 + 1) + 1 }
else { rand % max(k54, 1) + 1 }
```
Here rand = |GetTickCount + LCG rand15| (FUN_004d61ec), and the delay is in 1/10 s.

Events
- 0x01 (needs aux): +0x118 = k54, +0x120 = int1 > 0 ? int1 : -1, flags |= 0x200, resolve sounds, mode = int0.
- 0x10 tick (needs aux):
  - not running and mode == 0: deadline = now() + delay(); running = true.
    Free-running mode arms itself at level start.
  - running and now() > deadline and s9 is found: send 0x80 to s9 and play S4 at `other`.
    - If mode == 1: stop. Otherwise re-arm with a new delay().
    - Then, if shots > 0, shots -= 1. The count gates only mode-1 starts; mode 0 repeats forever.
- 0x40 tag (no edge test), 0x80 (edge test), 0x200 (player-ish + edge test): if `shots != 0 && mode == 1 && !running`,
  deadline = now() + delay() and running = true.
  These events have no effect in mode 0.

Data check (7): all have mode 0 and tag 9999, so they free-run from load.

| target | int2..int3 (tenths) | seconds |
|---|---|---|
| SIREN | 120..500 | 12–50 s |
| PLANE | 300..1000 | 30–100 s |
| CHOPPER | 500..2000 | 50–200 s |
| MONSTER1 | 600..3000 | 60–300 s |
| LYCAN | 100..1000 | 10–100 s |
| ZAPPER → FLASH1 (lightning) | 10..1000 | 1–100 s |
| FIXEDAIR (LIPS09 #182) | 2000..9000 | 200–900 s |

All are ambient-sound OSOUND targets apart from FLASH1. FIXEDAIR is **not found** in LIPS09, so that object
never fires (it stays expired and retries each tick). The 4 in LIPS09 have k54 = 2 (unused, because the
int range is valid).

### OCOUNT (index 72, FUN_0042b5cb, mask 0x2c1). Confidence H
Fires on every N-th event (N = k54) and then reloads.
- 0x01: counter (+0x118) = k54, flags |= 0x200, resolve sounds.
- 0x40 tag: count, with no edge test.
- 0x80: EDGE, then count.
- 0x200: other must pass FUN_0048a05a, and then the filter (int4 > 0 ? FUN_0042426a : FUN_0048a05a again).
  Then EDGE, then count.
- count: `counter -= 1; if counter < 1 { counter = k54; if aux && (t = find(s9)) { send t 0x80; play S4 at other; if int3 > 0 { ActivateTag(int3) } } }`.
  The counter reloads even when aux or the target is missing. Its own tag is never fired; only aux+0x94 is.

Data (1): LIPS14 'COUNTER' with k54 = 7 and s9 = STRTDONE (OTIMER, 4 s), which triggers OTAGSW DODONE.
It is fed 0x80 by two OEFFSW (tags 30/31).

### OEFFSW (index 73, FUN_0042b7d0, mask 0x2c1). Confidence H
A relay: it turns a tag, trigger or bump into a 0x80 on a named effect object. It does not fire a tag itself.
- 0x01: flags |= 0x200, resolve sounds.
- 0x40 (no edge test), 0x80 (EDGE), 0x200 (FUN_0048a05a(other) + EDGE): require aux and t = find(s9).
  - If flags & 0x1000 is clear: send t 0x80 and play S4 at other.
  - Otherwise, if k54 > 0: send t 0x80, k54 -= 1, and play S4.

Data (2): LIPS14 tags 30 and 31 lead to COUNTER (OCOUNT). Both are unlimited and have no sound.
Tag 30 is also heard by an OTEXT and an OINCOS.

### OSTRAP (index 29, FUN_0042ae89, mask 0x2002c0). Confidence M-H
A level exit ("strap" = load a new orb).
- 0x01: flags |= 0x200; resolve sounds if aux.
- 0x40 (no edge test), 0x80 (EDGE), 0x200 (FUN_0048a05a + EDGE). Requires aux. Then:
  1. pending_level = s9 (copied to global DAT_0070954c, the orb/level file name, e.g. "LIPS02").
  2. start_obj = s8 (DAT_00515e78). After the new level loads, the main loop FUN_004bb442 looks up an
     object with that name (FUN_00424862). If found, it puts the player there (FUN_0042d272 → FUN_0042ce2d),
     zeroes the player velocity and wakes the player. An empty name means the default start.
  3. Carry the player's state over (FUN_0042adaa: copy the names of inventory items whose item flags +0xf&8
     are set into a keep-list at DAT_0053d0e4. FUN_0044b16a: copy 16 ints from the player stats block
     (+0x1b4)+0x198 to DAT_0053d030).
  4. Loading text: s6 is looked up in a 16-byte name→string table at DAT_00567998 (FUN_0046e93e). If found,
     that string is used, else the default "Loading..." (DAT_00515bd8).
  5. DAT_00515d30 = 1 (a level change is pending; the main loop handles it at end of frame: unload with
     event 2, load the new orb, ActivateTag(9999), and so on).
  6. If int0 == 1, set DAT_00515508 = 1 (flush the sound bank on unload; the main loop also sets it
     unconditionally). Otherwise play S4 if it is set.
- 0x200000 is in the mask but ignored by the handler.

Data (17): one per level from LIPS00 to LIPS16. Each is named TOLIPSnn with s9 = 'LIPSnn+1' (the files
exist up to LIPS17), tag 500, no aux ints, and s6/s8 empty. Each is fired through tag 500 by a wall
WTAGSW, which is usually also named TOLIPSnn. LIPS07's has k54 = 2, which is unused.

---

#### [E] Sub-functions decoded
| addr | meaning |
|---|---|
| FUN_00489ff8(o) | "player-ish body": o && (flags&0x40000 \|\| flags&0x80 \|\| +0x10&0x200000) |
| FUN_0048a05a(o) | player-ish body itself, else if o is a projectile (flags&0x200000 or +0x10&0x80) whose owner +0x144 is player-ish, return the owner; else 0 |
| FUN_0049fc30() | game clock in 1/10 s (real time; updated from GetTickCount by FUN_0049fa1c, which also keeps 10 ms / 40 ms / 1 s counters) |
| FUN_0049e0bd(aux) | resolve aux sound-name slots 0..4 to Sound ptrs aux+0x9c..+0xac (skip "" / "NO_NAME"); set aux+4 bit0 = resolved |
| FUN_0049d933(o, snd, 1) | play a sound. When o is a positional object (+3&1) and the distance-limit sound option is on, skip it if o is farther than DAT_00567104 from the player |
| FUN_00472ac1(a, b) | ActivateTag for every tag a..=b (a >= 0) |
| FUN_00472b7c(t) | ActivateTag. Recursion depth is limited to 12. Runs ONTAG[t] (DAT_0056c1f4[t]) if present. Broadcasts 0x40 to objects (+0x44), walls (+0xa4) and regions (+0x84) with that tag, but only if t >= 999, t == 9999, or the tag is flagged as having listeners (DAT_0056bd8c[t]). Tags 8000..8999 also go to FUN_0048ee77 |
| FUN_00429254(o) | ORANTAGS random-tag picker (see above) |
| FUN_00423b0b(lo, hi) | random in [lo, hi]; 0 if hi - lo < 1 |
| FUN_004d61ec() | random = abs(GetTickCount() + rand15()) (Watcom LCG 0x41c64e6d/0x3039, FUN_004f013f) |
| FUN_00424862(name) | find an object by name (+0x58). FUN_00424668 then falls back to walls, regions, views, paths, ... |
| FUN_0042d272(o) | put the player at object o (start spot after a strap/teleport), zero velocity, wake |
| FUN_0046e93e(name) | case-insensitive lookup in the 16-byte table at DAT_00567998 (name[≤11] → ptr) |
| FUN_0049de12() | free all loaded sounds (sound-bank flush) |
| OEFFOFF FUN_00433547 / OEFFON FUN_00433215 | target = s9 via FUN_00424732. OFF: only if not already disabled; send 0x100000; set the disable bit unless the target effect is OTIMER. ON: clear the disable bit; send 0x200000 if the target mask has it, else 0x80 |


---

<!-- part F -->

The five handlers share the same activation gate. In this section it is called **GATE**:

```
GATE(self, ev, other):
  ev == 0x40                        -> pass (tag activation; no edge test)
  ev == 0x80                        -> pass if self is not in contact (self.+4 & 0x0f == 0)
  ev == 0x200 && playerish(other)   -> pass if self is not in contact
  any other event                   -> ignored (0x100, 0x200000, ... do nothing)
```

- The edge test reads the contact bits of the **effect object itself**: bits 1, 2, 4 and 8 of byte +4.
- `playerish` differs per handler:
  - OTELE uses FUN_00489ff8(other), which accepts the object itself only (flags 0x40000, flags 0x80, or +0x10 bit 0x200000).
  - The others use FUN_0048a05a(other), which accepts the player, or the owner when a player-owned projectile hits.
- Init (event 1) is the same in all five: OWTOW, OOTOO, OFLYON and OFLYOFF set flags |= 0x200 (immovable), and all five resolve the aux sounds (FUN_0049e0bd).

---

### OTELE (index 15, FUN_0042d2ff, mask 0x2003c0) — confidence H (code), M (net-team part)

**Trigger events**
- 0x01: resolve aux sounds and set self+0x124 = 0. Flags are not touched.
- 0x40: teleport the local player (DAT_00510778).
- 0x80: same as 0x40, after the edge test.
- 0x200: teleport `other`, if `other` is itself a player-type object (FUN_00489ff8) and passes the edge test. Projectiles do not trigger it.
- 0x100 and 0x200000 are in the mask but do nothing. The disassembly confirms this: every event other than 1, 0x40, 0x80 and 0x200 jumps to the exit. Most likely 0x200000 (from OEFFON) is in the mask only so the effect "accepts" re-enabling.
- The handler does nothing if self has no aux.

**Fields**
| rec | off | type | meaning |
|---|---|---|---|
| self | +0x54 | i32 | 0 = use named destination; ≠0 = **random region** (FUN_004d83cd) |
| self | +0x118 | ptr | scratch: destination found this activation (reset to 0 each time) |
| self | +0x34 | fx | used (probably a bug) as the "height" in the ceiling-clamp of the region path |
| aux | slot5/6/7 (+0x4a/+0x56/+0x62) | name | net game only (`*DAT_0056e69c & 1 or 4`): destination for team 1 / team 2 / other (activator +0x1ae short) |
| aux | slot8 (+0x6e) | name | key in INI section `[namedmessages]` (file path DAT_00708368). The 120-char text is shown as a message to the activator |
| aux | slot9 (+0x7a) | name | **destination**: tried first as an object name (FUN_00424862), then as a region name (FUN_0044e0ba) |
| aux | int0 (+0x88) | i32 | 1 or 3: activator drops all possessions at its position (FUN_0045e439 mode 3). 6 or 3: FUN_0047b61e(1,9,1) resets the carried "role/power" of players 1..9 (ext+0x20, fires tags 0x398+id and 900) |
| aux | int1 (+0x8c) | i32 | region path: 0 = keep relative position; ≠0 = region centre (z = floor + h/2); 2 = z = floor; 3 = z = ceiling − self.h − 2 |
| aux | int2..4 (+0x90..+0x98) | 16.16 | x, y, z offset added in the region path |
| aux | S4 (+0xac) | snd | played at `other` (the original EBX, even for tag events) |

**Behaviour**
```
on 0x40/0x80/0x200 passing GATE, with self.aux set:
  act = playerish(other) ? other : PLAYER
  if int0 in {1,3}: drop_all_possessions(act, act.point)
  if int0 in {6,3}: reset_player_roles(1..9)
  dest = null
  if netgame && self.+0x54 == 0: dest = find_obj(team slot 5/6/7 by act.+0x1ae); msg = slot8
  if !dest && self.+0x54 == 0:   dest = find_obj(slot9); msg = slot8
  if dest (object):
      MoveObjToObject(act, dest)                     // FUN_0042ce2d, see below
      morph = true
  else:
      if netgame && +0x54 == 0: dest = find_region(team slot)
      if !dest && +0x54 == 0:   dest = find_region(slot9)
      if !dest && +0x54 != 0:   dest = random_region_fitting(self)     // FUN_004d83cd
      if dest (region):
         src = act.region(+0xfc)
         if int1 == 0:
            if src and dest have the same bbox w/h and floor-to-ceiling height:
                 pos = dest.bbox_max - (src.bbox_max - act.xy);  z = dest.floor + (act.z - src.floor)
            else pos.xy scaled proportionally into dest bbox; z = dest.floor + (act.point.z - src.floor)
         else: pos.xy = dest.centre(+0xd4,+0xd8); z = dest.floor + (ceil-floor)/2
         if int1 == 2: z = dest.floor;  if int1 == 3: z = dest.ceil - self.h - 2
         pos += (int2, int3, int4)
         clamp z: if > ceil: z = ceil - self.h - 2;  if < floor: z = floor
         MoveObjToRegion(act, dest, pos)              // FUN_0042d19a
  if dest:
      wake(act)                                       // FUN_00467029
      if morph and dest.aux:                          // player takes on the template's aux
          stop_sounds(act.aux); act.aux = MergeAux(act.aux, dest.aux); resolve sounds
          act.aux.S0(+0x9c).+0x26 = 10
          if act.aux.int0 < 1: act.+7 &= ~6; DAT_0053d0d8 = 0       // footstep/bob timer off
          else: act.+7 |= 6; DAT_0053d0e0 = int(act.maxspd_x) * int0; DAT_0053d0dc = 0;
                DAT_0053d0d8 = 16000 / int0                         // same block as OSTEPON
      play self.aux.S4 at other
      if message text found: show it to act (FUN_00453ad4/FUN_00453951/FUN_004b149e)
      DAT_00515528 = 1     // "teleported" flag (also set by GOTO / FUN_0042d272)
  if act == PLAYER && PLAYER.ext: ext+0xd |= 4
```

**MoveObjToObject (FUN_0042ce2d)** is a teleport and morph to a template object:
- It copies the point x/y/z, z (+0x2c), +0x30 and the angle (+0x24) from the template.
- For player-type objects it also copies +0x118..+0x124.
- These fields are copied only when the template value is not "keep" (−1.0 / 0xffff0000):
  - height +0x34 (into +0x38 and +0x34)
  - radius +0x3c
  - tag +0x44
  - max speeds +0x7c..+0x88 (players also get the DAT_0056e6b4[] bonus added)
  - accel +0x8c..+0x98
- For players it then calls FUN_0045a445.
- If the template's sprite (+0x64) is non-zero, it copies the 6-dword sprite block.
- It copies the flying flag (flags & 2) in both directions.
- It relocates the object to the region found by point location (FUN_004d89c5). If no region is found, it calls FUN_004762af(−6).

**Edge cases**
- The effect is repeatable and has no cooldown, apart from the GATE edge test (re-arms once contact ends).
- Self is never hidden or destroyed.
- If no destination resolves, nothing happens: no sound and no message.
- +0x54 ≠ 0 skips the named lookup completely.
- The random region must fit self, not the activator:
  - bbox width and height ≥ 2·self.radius
  - ceiling − floor ≥ self.h + 2
- The random search starts at rand % (n−1) and wraps around.

**Data check**
- 45 records. 43 have an aux; LIPS00 #685 and #686 have none and are inert.
- int0..int4 are all 0, except LIPS00 #555 (int0 = 125, which has no meaning here).
- Slots 5–8 are empty in every record. So there are no team destinations and no named messages.
- +0x54 ≠ 0 only on LIPS00 'FLUSHER1' (k54 = 50). That toilet therefore sends the player to a **random region**, even though s9 = RBIRTH01.
- Slot-9 destinations:
  - 40 are **OSPOT objects**. Examples: RBIRTH01..05 (respawn spots used by every 'DEATH01'/DETHn object with tag 3/75–78), UPSTAIRS, HIGHER01, SMART1, OPENER01, WARP01, WHIZZER1, CHEIRAFT, SWIMMER1/2, GIANT.
  - 2 are regions: LIPS01 REST01 and PARK01. These are tag-driven teleporters (tags 105/106), so they use the relative or proportional mapping.
  - 1 is missing: LIPS03 #268 → 'CHECKER'. The name exists only in that aux, so this teleporter is dead.
- The destination OSPOTs are **player templates**:
  - Typical template: flags 0x1000230, h = 48, r = 16, speeds 800/800/200/853, accel 400/400/400/2275, the player sprite, aux int0 = 32, s0 = STEP01.
  - So a teleport also re-applies the normal player stats.
  - LIPS07 GIANT (h 640, r 128, speed 500, int0 = 64, s0 = BREATH06) turns the player into a giant.
  - LIPS00 SWIMMER1 (flags | 2 = flying, h 32, speed 300/300/150, s0 = DROPE03) makes the player swim.
  - LIPS08 RBIRTH02 has no aux, so the aux is not merged.
- This fits the code: OTELE is the respawn and morph mechanism.

---

### OWTOW (index 67, FUN_00426cba, mask 0x2c1) — "wall to wall" appearance copy — confidence H

This effect is not a teleport. It copies the look of one wall onto another, for example "door closed → open" or "light → dark".

**Trigger events**
- 0x01: init.
- 0x40, 0x80, 0x200 (GATE): apply the copy.
- If self.+4 bit 0x20000000 (destroyed) is set, every event is ignored.

**Fields**
| rec | off | meaning |
|---|---|---|
| aux slot9 (+0x7a) | target wall name (FUN_00458009 = find wall by name, wall+0xb0) |
| target wall's own aux slot7 (+0x62) | preferred source wall name |
| aux slot8 (+0x6e) | source wall name, if the target's aux has no slot 7 |
| aux slot5 (+0x4a) | if non-empty, the target wall is **renamed** to it |
| aux S4 | played at `other` |

**Behaviour**
```
tgt = find_wall(slot9); if !tgt: return
src = (tgt.aux ? find_wall(tgt.aux.slot7) : null) or find_wall(slot8); if !src: return
keep = tgt.{p1 +0xc, p2 +0x10, front +0x80, back +0x84, +0xd4, xoff +0x9c, yoff +0xa0, len +0x8c, aux +0xdc}
CopyWallLook(tgt, src)              // FUN_00426b9a
restore keep
tgt.aux = MergeAux(tgt.aux, src.aux) or old; tgt.+0xd8 = tgt.aux.index
if slot5 != "": tgt.name = slot5
play S4 at other
FUN_00426b1f(tgt)                   // reset animation of upper/middle/lower textures
```

**CopyWallLook (FUN_00426b9a)** copies these fields from the source wall:
- the effect index (+0xd0)
- flags +0x00 and +0x04, so the target also takes the source's solid and passable state
- the 3 texture blocks (+0x2c, +0x44, +0x5c, 24 B each)

It also copies these fields when the source value is not −1:
- +0x90
- light +0xa4 and +0xa6. If global DAT_00510788 is set, light −= 2.
- +0xac

**Edge cases**
- The effect is repeatable. Applying it again re-copies the same source.
- The OPN/CLS pairs are one-way. To go back, a level would need a second OWTOW object.
- Renaming through slot 5 could build chains, but no shipped record uses slot 5.

**Data check**
- 45 records. All ints are 0; the only exception is LIPS13 #266 (int4 = 50, unused).
- Slot 5 is empty in every record.
- No target wall has an aux with slot 7, so the source is always slot 8.
- Example pairs: FRONTOPN→FRONTCLS, DOR1OPEN→DOR1LOCK, LIGHT0n→DARK0n (8 objects on tag 93 in LIPS00), GATxxOPN→GATxxCLS, MGATE*OP→MGATE*.
- Most sources are plain walls (effect −1); some are WTAGSW walls. Most targets are WTAGSW walls, so the effect index of the target changes (e.g. a closed switch-door becomes a plain open wall).
- Missing names, which make those records no-ops:
  - LIPS03 #237: ALCHOPN and ALCHCLS (both missing)
  - LIPS14 #24: MGATE17O and MGATE17
- LIPS03 'D84OPN' names 3 walls. find_wall takes the first one.

---

### OOTOO (index 66, FUN_00427c70, mask 0x40002c1) — "object to object" morph — confidence H (M for the 0x4000000 sender)

**Trigger events**
- 0x01: flags |= 0x200; resolve aux sounds.
- 0x40, 0x80, 0x200 (GATE): apply the morph.
- 0x4000000: apply, with no edge test (like 0x40). No sender of 0x4000000 was found in the code; it may be unused.

**Fields**
| rec | off | meaning |
|---|---|---|
| aux slot9 | target object name |
| aux slot8 | source (template) object name |
| target's aux slot7 | fallback source name, used when slot8 is not found |
| aux slot5 | if non-empty and not "MINUSONE", the target is renamed to it |
| aux int0 | decimal digits. Units: 1 = only the first object with the target name; else **all** objects with that name (FUN_00423bde). Tens: >0 = also copy the ObjectState (FUN_00427efe). Hundreds and thousands are computed but unused |
| aux S4 | played at each target |

**Behaviour**
```
tgt0 = find_obj(slot9); if !tgt0: return
src  = find_obj(slot8) or (tgt0.aux ? find_obj(tgt0.aux.slot7) : null); if !src: return
for tgt in (units==1 ? [tgt0] : all objects named slot9):      // via FUN_00423bde
    MorphObj(self, tgt, src)                                    // FUN_004278b1
    if tens > 0: CopyObjState(self, tgt, src)                   // FUN_00427efe
if tgt0.+0x10 & 0x400: FUN_004489f6(tgt0, other, 99)            // ObjectState visibility refresh
```

**MorphObj (FUN_004278b1)**
1. If the target is not the player, remove it from the active list (FUN_004729ae).
2. Save the target's:
   - point
   - +0x1bc (unless src+6 bit 0)
   - z (+0x2c) and +0x30
   - region list link (+0x14c) and region (+0xfc)
   - aux
   - effect index
   - angle
   - name
3. Stop the target's aux sounds, then play self S4 at the target.
4. Copy the object type from src (CopyObjType, FUN_0042759b).
5. Restore the saved fields. The point's flag dword is taken from src; +0x198 is taken from src.
6. Rename the target to self slot5, if that slot is set.
7. Set target.aux = MergeAux(target.aux, src.aux) and target.+0x1ac = aux index.
8. If the target is the player:
   - restore angle and effect (it stays OPLAY)
   - set flags |= 0x40000 and point flags |= 3
   - call FUN_0045a445
   - re-resolve sounds, and set S0.+0x26 = 10
   - recompute the footstep timer from aux int0, the same way as OTELE
9. Otherwise, re-register the target in the active list for its new effect (FUN_0047291d).
10. If src flags & 0x800: FUN_0048eddd(src.name) shows a HUD/inventory notice.
11. Wake the target.

**CopyObjType (FUN_0042759b)** copies these fields from src:
- effect +0x140
- flags +0x00 and +0x04
- +0x1d0
- clears +0x23 bit 0x10
- sprite block +0x64 (6 dwords)
- +0x170..+0x18b
- +0x118..+0x124, but only if the target is the player or both objects have +0x10 & 0x10

It also copies these fields when the source value is not "keep" (−1 or −1.0):
- +0x54
- angle
- radius
- height (+0x34 into +0x34 and +0x38)
- +0x40
- tag +0x44
- +0xdc..+0xf8
- speeds +0x7c..+0x88 (player bonus added) and accel +0x8c..+0x98
- +0x46

**CopyObjState (FUN_00427efe)**
- Copies 0x1d8 bytes of src's ObjectState (+0x1b4) into the target's ObjectState, allocating one (FUN_004d7317) if the target has none.
- Sets state+0x9c = target and state+0x10 = target index.
- Copies target.+0x10 = (old & 3) | (src & ~3).

**Edge cases**
- The effect is repeatable and the self object stays.
- The target keeps its position but takes the source's look and behaviour. For example, an invisible effect-less placeholder becomes a working OPICKUP.
- Self is not disabled after use. Because the target keeps its name, firing again re-morphs it into the same thing.

**Data check**
- 22 records. int0 = 0 in every record, so the morph always applies to all objects with the target name and never copies the ObjectState.
- Every s8 and s9 name exists as an object in the same level. No target aux has slot 7.
- Typical uses:
  - **reveal a pickup**: s9 is a placeholder with effect −1 (NOBRONZE, NOGOLD, NOTHING, NOGREEN, FLOWERZ, NOTLAST, NOFART, NADA, NOTDOG, KEYAOFF, APPLEOFF); s8 is an OPICKUP template (BRONZE, GOLD, SUMTHING, ...)
  - **switch flip**: s9 is an OTAGSW (SWXXOFF, SW1OFF, ...); s8 is a plain sprite object with effect −1. The switch changes sprite and becomes inert.
  - **player morph** (s9 = 'PLAYER'):
    - LIPS00 #688 WATERGUY: h 16, speed 500/500/100, s0 BREATH06
    - LIPS00 #689 RESTORE4: normal player stats
    - LIPS03 #64 SKELW1
- LIPS02 'TAPESTRY' has k54 = 20, which the handler ignores.

---

### OFLYON (index 26, FUN_00429d26, mask 0x2c0) — confidence H

**Trigger events**
- 0x01: init.
- 0x40, 0x80, 0x200 (GATE): apply.
- The actor is PLAYER for 0x40 and 0x80, and FUN_0048a05a(other) for 0x200.

**Fields**
| rec | off | meaning |
|---|---|---|
| aux int0 | if > 0: actor height +0x34 = int0. The value is stored **raw** (not <<16) |
| aux int1..int4 | ext fly speeds, each <<16 unless −1: int1 → ext+0x204 (up), int2 → ext+0x208 (down), int3 → ext+0x20c (idle sink), int4 → ext+0x214 (fall). All four are skipped if all are 0 |
| aux S4 | played at other |
| ext+0x22c | set to self (the active fly effect object) |

**Behaviour**
```
(dead code: if actor == null copy velocities +0xac.. to +0xbc..; real save is skipped, crash path)
actor.vz(+0xb4) = 0; actor.flags |= 2 (flying)
if self.aux:
   play S4 at other
   if int0 > 0: actor.h = int0 (raw)
   if actor.ext:
      stop S0..S4 of previous ext+0x22c effect object (this can be self, which cuts off the S4 just started)
      if any(int1..int4 != 0): set the fly params (−1 = keep)
      ext+0x22c = self
else:
   actor.h -= frame_scaled(self.accelZ +0x94) / 3, but not below FullHeight(+0x38)/3
DAT_00510ce0 = 0   // airborne phase did not start with a jump
```
- Compare the console command FLYON (0x409adf): it does save the velocities into +0xbc..+0xc8, and sets ext+0x20c = maxspd_z/4 or 0.
- Repeatable; nothing is disabled.

**Data check**
- 19 records. 17 are the standard 'FLYON' object: tag 1, no aux, flags 0x1000230. They take the no-aux path, which only shrinks height a little and sets flying. They are presumably fired by ActivateTag(1) from a fly key or cheat (not traced).
- LIPS07 #37: tag 45, ints all 0, S4 = POP01. This just sets flying and plays POP01.
- LIPS10 #49 'FLYON' has an aux with all-zero ints.

---

### OFLYOFF (index 27, FUN_0042a0da, mask 0x2c0) — confidence H

**Trigger events**
- 0x01: init.
- 0x40, 0x80, 0x200 (GATE): apply, but only if an actor exists and actor.flags & 2 (it is flying).

**Behaviour**
```
actor.vel(+0xac..+0xb8) = actor.saved_vel(+0xbc..+0xc8)    // saved by console FLYON; not by OFLYON
actor.flags &= ~2
if self.aux: play S4 at other; if int0 > 0: actor.h = actor.FullHeight(+0x38)
else:        actor.h = actor.FullHeight
if actor.ext:
   stop S0..S3 of ext+0x22c effect object (S4 is not stopped); ext+0x22c is not cleared
   ext+0xc &= ~0x0360       // clear fly-up/down anims (0x20,0x40) and ext+0xd bits 0x01/0x02
DAT_00510ce0 = 0
```
- The fly speeds set by OFLYON are not restored.

**Data check**
- 19 records. 17 are the standard 'FLYOFF' or unnamed object: tag 2, no aux.
- LIPS07 #9 'DOWNWEGO': aux with ints 0, S4 = POP01, k54 = 50 (k54 is unused).

---

#### [F] Shared helpers decoded
| addr | meaning |
|---|---|
| FUN_00424862(name) | find **object** by name (obj+0x58); "" → null |
| FUN_0044e0ba(name) | find **region** by name (region+0xa4) |
| FUN_00458009(name) | find **wall** by name (wall+0xb0) |
| FUN_00424911(name) | find by name in the list DAT_00567738 (name at +0x44; probably Points or Paths) |
| FUN_00423bde(self, name, cb, arg) | call cb(EAX=self, EDX=obj, EBX=arg) for every object with that name. If an object has +0x1d0 bit 0x80 or +0x1d1 bits 2/0x40 (group objects), iterate its group (FUN_004739c6) instead |
| FUN_00423dfb(dst, src) | **MergeAux**. Allocates dst if null (FUN_004d79a5). Copies every name slot unless src is "-1" or "MINUSONE" (an empty string *does* copy). Copies ints unless −1, and sound pointers unless their slot is "-1" or "MINUSONE". Clears dst+4 bit 0 so sounds are re-resolved. Returns 0 if src is null |
| FUN_0049e0bd(aux) | resolve aux sound slots 0..4 into Sound pointers (once, sets aux+4 bit 0) |
| FUN_0049d538(aux) | stop all 5 aux sounds (FUN_0049d489 = stop one sound) |
| FUN_0049d933(obj, snd, 1) | play a sound at an object, with network distance gating |
| FUN_0042ce2d(obj, tmpl) | MoveObjToObject: teleport plus morph-to-template (see OTELE) |
| FUN_0042d19a(obj, region, pos) | MoveObjToRegion: set xy/z. If inside the region bbox, relink it into the region; else clamp z (player) |
| FUN_004d83cd(obj) | random region in which obj fits (FUN_004d82fd: bbox ≥ 2r, height ≥ h+2) |
| FUN_004d9572(obj, region) | is obj's x, y, z inside the region's bbox and floor/ceiling |
| FUN_0042759b(dst, src) | CopyObjType (see OOTOO) |
| FUN_004278b1 / FUN_00427efe | OOTOO MorphObj / CopyObjState callbacks |
| FUN_00426b9a(dst, src) | CopyWallLook (see OWTOW); FUN_00426fc7 is the region equivalent |
| FUN_00426b1f(wall) | reset the texture animations of a wall's 3 textures (FUN_004a0c61) |
| FUN_0045e439(obj, pt, mode) | drop all possessions (FUN_0045e1f4 per item = DropOnePossession) |
| FUN_0047b67a(player, 1) | clear the player's ext+0x20 role/power, fire tags 0x398+id and 900 |
| FUN_00489ff8(obj) | is player-type (flags 0x40000, flags 0x80, or +0x10 bit 0x200000). No owner lookup |
| FUN_00473195(v) | scale a 16.16 value by the frame step (table DAT_0056e470 or v·DAT_005670e8) |
| FUN_0042714f | same pattern as OWTOW for **regions** (region-to-region copy, FUN_00426fc7, find_region) — probably ORTOR |
| console table 0x50fa48 | FLYON/FLYOFF/GOTO/... console commands (FLYON 0x409adf saves velocities to +0xbc) |

#### [F] Open questions
- Who sends event 0x4000000 (only OOTOO's mask has it)? No sender was found.
- Who fires tags 1 and 2 (the standard FLYON and FLYOFF objects)? Probably the fly cheat or key.
- What reads DAT_0053d0d8/dc/e0 (footstep/head-bob timer)? It is likely accessed through a struct base; the OSTEPON analyst should cover it.
- OTELE's region path clamps z with **self**.h, not the activator's height. This looks like an original bug.
- OFLYON's velocity save into +0xbc only runs when the actor is null (dead code). So OFLYOFF restores whatever the console FLYON last saved, or zeros.


---

## 3. Consolidated open questions

1. **Senders still unknown:**
   - who sends event 0x4000000 (only OOTOO declares it);
   - what fires tags 1 and 2, which drive the standard FLYON/FLYOFF objects (probably the fly key or a cheat).
2. **The activator for ActivateTag (EDX)** is stale in several callers (OPICKUP, ODROP, OSELTOOL). Treat
   `other` on event 0x40 as unreliable.
3. **Runtime flag meanings not decoded:**
   - object `+0x10` bit 0x20000000 and `+0x1d0` bit 0x80000000 (set on every pickup; only a map-drawing
     reader was found);
   - OORIENT flags 0x4000/0x8000;
   - OHOTSPOT `+0x1d1` 0x20;
   - OSOUND `+0x0c` 0x10;
   - which renderer code reads View bit 2 (OVIEWON).
4. **The mover-script action word** at object +0x20 (OTHING/OWALK/OFOLLOW). Only these bits are confirmed:
   - 0x10000000 destroy;
   - 2 fire;
   - 0x4000000 idle;
   - 0x20000000 drives the player.

   The full bit set belongs to the mover/physics doc.
5. **OSPAWN has no consumer.** The only candidate is the arrival-object lookup after a level change
   (`DAT_00515e78`, from OSTRAP s8), and all shipped OSTRAPs leave s8 empty.
6. **Original bugs and quirks** (should a reimplementation copy them?):
   - OTELE's region path clamps z with the teleporter's height instead of the player's;
   - OSETF's busy bit can stay set after a queued move that finishes immediately;
   - in OWALK's 0x40 branch, the state-block pointer is an uninitialised local;
   - when OPICKUP fails because the inventory is full, it fires its tag again on every new contact.
7. **Dead data:**
   - LIPS09 #182 ORANDOM targets FIXEDAIR, which does not exist;
   - LIPS03 #268 OTELE targets 'CHECKER', which does not exist;
   - OWTOW LIPS03 #237 and LIPS14 #24 name walls that do not exist;
   - ODROP points KEYRET (LIPS03) and BRNSBAK (LIPS06) do not exist, so the item falls back to the ODROP
     object's own position;
   - LIPS12 #18 OTAGSW fires tag 54, which nothing listens to.
8. **OTEXT list duplication:** the XRB already holds the OTEXT list (ObjTable512 plus a count) and INIT
   appends to it. It is not verified whether the count is reset first.
9. **Overlay fields:** the blit mode (+0x2c) and the x/y order of overlay +0x24/+0x28 conflict with
   XRB-records.md §11.
10. **Wall tag offset:** ActivateTag and wall collision read the wall tag as the i16 at +0xa6 (the
    `int >> 16` of the dword at +0xa4). XRB-records.md §3 lists +0xa6 as "-1 usually, sometimes 7;
    maybe a lit level". It is in fact the wall's tag (-1 = none), and the light is the low i16 at +0xa4.
    Fix §3 there.
