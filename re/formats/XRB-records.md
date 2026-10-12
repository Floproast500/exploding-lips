# XRB record layouts (Points, Walls, Regions, Objects, Texs, ...)

Companion to `XRB.md` (file header, descriptors, Texs/Pics basics). Clean-room spec: these are
descriptions written from reading the loader, the post-load pointer fix-up and the
renderer/collision code, and they are checked against all 19 shipped levels by
`re/scripts/xrb_validate.py` (all hard checks pass). Parsing helper: `re/scripts/xrb_lib.py`;
`re/scripts/xrb_dump.py` now uses it.

Confidence column:
- **code**: confirmed in code (function given)
- **data**: inferred from data, holds across all levels (validator check, or distribution)
- **guess**: plausible, not verified

Key functions:
- `FUN_004d5968` ReadCompiledOrb (loader)
- `FUN_004d21fa` SetOrbPointers, the post-load fix-up that turns every index field into a pointer.
  This gives the type of every reference field below. The index value -1 becomes NULL.
- `FUN_004d89c5` / `FUN_004d8d78` find the region containing a point
- `FUN_004a4a1f` UpdateObject (movement and wall collision)
- `FUN_004a102a` player movement
- `FUN_0041592e` wall column renderer ("Draw_cw")
- `FUN_004187f8` floor/ceiling span setup; it is called from `FUN_00418c27`, which is reached
  from `FUN_00418225` (ceiling) and `FUN_00418376` (floor) in `decomp_extra.c`
- `FUN_004d6b0e` CloneRegion; its debug string names fh/ch and wall f/b
- `FUN_004a09fe` / `FUN_004a0c61` texture animation stepping
- `FUN_004a0375` per-frame view ray setup

## 0. Conventions (units, handedness)

- **Fixed point.** Coordinates, heights, lengths and offsets are signed 16.16 (`i32 / 65536`).
  Shipped maps only use integer values.
- **Axes.** X points right (east) and Y points up (north) in the top view. Z is up: floor < ceiling,
  larger is higher.
  - Evidence: the floor span code (`FUN_004187f8`) maps world X to texture u and **−Y** to texture v.
    It builds the view ray as (cos θ, −sin θ) with θ = −angle ± fov/2.
  - Wall orientation (below) gives the same result.
- **Angles.** u16 with 2048 units per full turn; code masks them with `& 0x7ff`.
  - MATH.DAT holds 2048-entry 16.16 sine (@0) and cosine (@0x2000) tables (`FUN_00480b8f`).
  - Angle 0 faces +X. Angles increase counter-clockwise (toward +Y), so the forward vector is
    (cos a, sin a). Evidence: the view rays in `FUN_004a0375` and `FUN_004187f8`, and player motion
    in `FUN_004a102a`. The player's usual start angle of 512 faces +Y.
- **Units.** One map unit is one texel.
  - Walls: u = t·length + xoff, where t ∈ [0,1] along the wall; v advances one texel per world unit.
  - Flats: u = x + xoff, v = −y + yoff.
  - Typical sizes: player height 48, radius 16; room heights 128–384; textures 64–256 px.
- **Light.** Light levels are rows of the shade table: 0 is brightest, larger is darker.
  - Distance shading adds dist / view+0x38, unless the region/vdraw flag bit 0x04000000 is set.
  - The result is clamped to [0, nShades−1].
- **Effect index.** Fields marked "effect" index the effect-command table, which uses the
  registration order of `re/engine/effects.md` (0 = OPLAY).
  - Data check: player = 0 (OPLAY), things = 13 (OTHING), walls 203/206 (WANIM/WTAGSW),
    regions 214/219/223 (RANIM/RTDOOR/RTAGSW).
  - At load, an entry is put on the active list if its flags have 8 or 0x10 (`FUN_0047291d`).
- **Texture slot (TC, 24 bytes).** Used by walls (×3), regions (×2), objects (sprite) and overlays.
  The sky also uses one (Tail).

  | +off | type | meaning | conf |
  |---|---|---|---|
  | 0 | i32 | Tex index (→ pointer) | code (fix-up) |
  | 4 | ptr | runtime: current anim entry (fix-up sets it to tex.entries) | code |
  | 8 | i32 | Pic index of the current frame (→ pointer); compiled value = a pic of that tex | code + data |
  | 0xc | i32 | runtime: frame timer, 16.16 ticks, counts down by frame time (stale in file) | code `FUN_004a09fe` |
  | 0x10 | i32 | extra delay added at each step (0 in data) | code |
  | 0x14 | u8 | runtime: mirror flag of the current pic | code |

  The renderer only needs `pic` for static textures. For animated ones, step the Tex sequence (§1).

## 1. File layout correction (loader order)

The sections are **not** simply contiguous in descriptor order. The real order
(`FUN_004d5968`, implemented in `xrb_lib.Xrb`) is:

1. Texs (alloc×100) + per-tex data
2. Pics
3. Points
4. Paths
5. Regions
6. Walls
7. Objects
8. ObjectStates
9. **Globals6x64** (0x180 bytes: 6 arrays × 16 object indices → `DAT_0053fbe4`…, fixed up to objects)
10. **ObjTable512** (0x800 bytes: 512 object indices → `DAT_0052f618`)
11. Views
12. Auxs
13. Sounds
14. **SampleTable** (hdr+0x40 records × 0x3c)
15. Texts (+ per-text data)
16. Windows (alloc×112, skipped)
17. Overlays
18. ActionNodes
19. **RegionWallList** (hdr+0x24 bytes)
20. **Tail** (0x7c bytes → `DAT_005676ac`)

With this order, the computed size equals the file size for all 19 files.

Notes on the extra blocks:
- **RegionWallList.** u32 wall indices, turned into wall pointers by the fix-up.
  - The number used is hdr+0x20 (= sum of region wall counts).
  - The bytes allocated are hdr+0x24 (= 8 × allocated walls).
- **SampleTable.** 60-byte records with `u32 flags`, name (char[]) @0x0c and i32 @0x2c.
  Their names and flags match the Sounds records, so this is likely a table of unique samples. data/guess
- **ObjTable512.** Sparse; non-(-1) entries are objects such as lamps and doors.
  Possibly a lookup by object number or tag. guess
- **Globals6x64.** Only the first 6 entries are used: 6 consecutive unnamed objects at the end of the
  object list. Likely per-player-slot objects. guess

Header fields copied out by the loader:

| hdr off | meaning | conf |
|---|---|---|
| 0x20 | number of used RegionWallList entries | code + data |
| 0x24 | byte size of RegionWallList | code + data |
| 0x40 | SampleTable record count | code |
| 0x44 | initial view index → current view `DAT_005675a0` | code |
| 0x48 | → `DAT_005100a4` (unknown) | code |
| 0x4c | start-object index → `DAT_0050ffc4`; a copy of the player (same point and angle) used as the respawn/start spot | code + data |
| 0x50 | → `DAT_00543cfc`; 1 in all levels | code |
| 0x54, 0x58, 0x60 | → `DAT_00510a88`, `DAT_00510a84`, `DAT_00510a8c`; 0 in data | code |

Tail (0x7c):

| off | meaning | conf |
|---|---|---|
| 0x08 | level title, char[16], e.g. "Mortuary - I 142" | data |
| 0x20..0x2c | 16.16 defaults (200, 200, 400, 568/1137) | guess |
| 0x30 | TC #1 (tex @0x30, pic @0x38), -1 in data | code (fix-up) |
| 0x48 | TC #2 = **sky/background** (e.g. BACK4W 640×200) | code (fix-up), data |
| 0x78 | Sound index = level music | code (fix-up), data |

## 2. Points (100 B)

| off | type | meaning | conf | evidence |
|---|---|---|---|---|
| 0x00 | u32 | type: 0, or 3 for named marker points | data | |
| 0x08 | u16 | own index | data (validator) | |
| 0x0a | u16 | reference count: number of wall endpoints using it, 1 for object points | data (validator) | |
| 0x0c | i32 | -1 or small index (400× 0); unknown | guess | |
| 0x14 | 16.16 | X | code | `FUN_004d4cfe`, `FUN_004d89c5` |
| 0x18 | 16.16 | Y | code | same |
| 0x1c | 16.16 | Z (objects: equals object z) | code + data | `FUN_004a4a1f` |
| 0x20 | i32 | runtime (render/transform scratch; compared to a clip value) | code | `FUN_0041592e` |
| 0x34 | 16.16 | 0/512/1024/1536 on some points; unknown, maybe an angle in the high half | guess | |
| 0x3c | i32 | Point index (→ ptr): next point; only path points | code (fix-up) | |
| 0x40 | i32 | Path index (→ ptr) | code (fix-up) | |
| 0x44 | char[12] | point name (spaces or e.g. "BLUE") | data | |
| 0x50 | i32 | -1 / 0 | guess | |

The loader also tracks the min/max of X, Y, Z over all points (`DAT_00598408..1c`).

## 3. Walls (240 B)

A wall is a 2-D segment p1→p2. Its **front region lies to the right of p1→p2** (X right, Y up), so
front-side loops run clockwise. The back region is on the left.

| off | type | meaning | conf | evidence |
|---|---|---|---|---|
| 0x00 | u32 | flags (see below) | code/data | |
| 0x04 | u32 | 0 (rarely 8) | | |
| 0x08 | i32 | own index (code reads it as u16 @0x08) | data | debug prints |
| 0x0c | i32 | Point p1 | code | fix-up, `FUN_004d89c5`, renderer |
| 0x10 | i32 | Point p2 | code | same |
| 0x14..0x28 | i32×6 | more Point refs; only 0x14/0x18 used (~100 walls), usually -1 | code (fix-up); meaning unknown | |
| 0x2c | TC | **upper** texture: tex @0x2c, pic @0x34 | code | `FUN_0041592e` |
| 0x44 | TC | **middle** texture: tex @0x44, pic @0x4c (one-sided walls; fences on two-sided walls) | code | same |
| 0x5c | TC | **lower** texture: tex @0x5c, pic @0x64 | code | same |
| 0x80 | i32 | **front region** (never -1) | code | CloneRegion "f=", `FUN_004d89c5` |
| 0x84 | i32 | **back region**, -1 = solid one-sided wall; may equal front ("internal" wall inside one region, e.g. grates; skipped for point location) | code + data | CloneRegion "b=" |
| 0x88 | i32 | third Region ref; only 12 walls; equals some object's +0x100 | code (fix-up); meaning unknown | |
| 0x8c | 16.16 | wall length = \|p2−p1\| (validator: all walls) | code + data | used as the u scale |
| 0x90 | i32 | used by collision when flag 0x4 is set (0 in data) | code | `FUN_004a4a1f` |
| 0x94 | 16.16 | rare values (8, 44, 56, ...); unknown | guess | |
| 0x9c | 16.16 | **texture x offset** (u = t·len + xoff); default 1.0 | code | `FUN_0041592e` |
| 0xa0 | 16.16 | **texture y offset** | code | same |
| 0xa4 | i16 | **wall light** (shade row; typically 8) | code | same |
| 0xa6 | i16 | -1 usually, sometimes 7; maybe a lit/alternate level | guess | |
| 0xa8 | i16 | 8 typically = 0xa4; likely the base light restored by light effects | guess | |
| 0xaa | i16 | colormap/translucency table index; 0 = normal shading | code | same |
| 0xb0 | char[8] | wall name/tag string (e.g. "WTOP...") | data | |
| 0xbc, 0xc0 | | rare non-zero; unknown | | |
| 0xd0 | i32 | effect index (WANIM/WTAGSW/...), -1 = none | code (fix-up) | |
| 0xd4 | i32 | always -1 | | |
| 0xd8 | u16 | copy of the aux index (0xffff = none) | data | |
| 0xdc | i32 | Aux index (→ ptr) | code (fix-up) | |

Wall flags (u32 @0):

| bit | meaning | conf |
|---|---|---|
| 0x2 | **solid**: blocks movement | code `FUN_004a4a1f` ("WallCol ... NOT solid" when clear) |
| 0x1 | set on passable two-sided walls (0x201) | data/guess |
| 0x4 | skipped by point-location in one mode; special collision with +0x90 | code |
| 0x200 | **two-sided** (back region present); exact on all walls | data (validator) |
| 0x400 | fill with a flat colour when no middle pic | code `FUN_0041592e` |
| 0x8000 | special middle-texture path (masked/see-through?) | code/guess |
| 0x10000, 0x20000 | change the collision test | code |
| 0x02000000 | always set | data |
| 0x08000000 | runtime "drawn this frame" | code |

Rendering notes (`FUN_0041592e`):
- If the vdraw "back-side" bit is set, the front and back regions swap, so the viewer's region is
  always treated as the front.
- Upper texture: drawn between the front and back ceilings. Bottom-aligned to the back ceiling:
  v = yoff + (backCeil − frontCeil) − picHeight at the front ceiling line.
- Middle texture: v = yoff, starting at the top of the opening.
- Lower texture: v = yoff, starting at the back floor line.
- Column index = (u & (pic.cols − 1)) << pic.colShift.

## 4. Regions (308 B)

A region is a convex-or-not polygon sector. It is bounded by the walls in its RegionWallList slice
and has flat floor and ceiling.

| off | type | meaning | conf | evidence |
|---|---|---|---|---|
| 0x00 | u32 | type/flags (0x10000080, 0x10000000, ...) | | ShowRegion "Type" |
| 0x04 | u32 | flags 2 | | |
| 0x08 | u16 | own index (u16 @0x0a is 0xffff) | code | debug prints, AddRegion |
| 0x0c | 16.16 | **floor height** (current) | code | CloneRegion "fh", point-location, collision |
| 0x10 | 16.16 | **ceiling height** (current) | code | CloneRegion "ch" |
| 0x14 | 16.16 | rest/designed floor height | data | |
| 0x18 | 16.16 | rest/designed ceiling height; differs from 0x10 only for closed RTDOOR regions (ceil = floor) | data (validator) | |
| 0x1c | TC | **floor** texture: tex @0x1c, pic @0x24 | code | fix-up, `FUN_00418376` |
| 0x34 | TC | **ceiling** texture: tex @0x34, pic @0x3c; pic -1 = no ceiling drawn (sky?) | code | `FUN_00418225` |
| 0x4c, 0x50 | 16.16 | floor texture x/y offset | code | `FUN_004187f8` |
| 0x54, 0x58 | 16.16 | ceiling texture x/y offset | code | same |
| 0x5c | i32 | Region below (rBelow): floor is a see-through portal into it | code + data | fix-up, `FUN_0041592e` |
| 0x60 | i32 | Region above (rAbove): ceiling is a portal; 0x5c/0x60 links are mutual | code + data | same |
| 0x64, 0x68 | i32 | more Region refs, -1 in data | code (fix-up) | |
| 0x6c..0x78 | i32/16.16 | effect parameters "Vars A–D"; for RTDOOR, C = open height | data | ShowRegion string |
| 0x82 | i16 | **floor light** | code | `FUN_004187f8` |
| 0x84 | i16 | **ceiling light** | code | same |
| 0x86 | u16 | 0xffff (set by AddRegion) | code | `FUN_004d684c` |
| 0x88, 0x8a | i16 | = floor/ceil light in data; likely base levels for light effects | guess | |
| 0x8c | i16 | floor colormap index (0 = normal) | code | `FUN_004187f8` |
| 0x8e | i16 | ceiling colormap index | code | same |
| 0xa0 | i32 | rare (50.0, 200.0, ...); effect parameter? | guess | |
| 0xa4 | char[8] | region name/tag ("SECRET1", "LIFT", ...) | data | |
| 0xb0 | i32 | effect index (RTDOOR, RTAGSW, RANIM, ...), -1 = none | code (fix-up) | |
| 0xb4 | i32 | first Object in this region; linked through object +0x14c | code + data | fix-up, `FUN_004d85a1` |
| 0xb8 | ptr | runtime: pointer to its RegionWallList slice (= list + 0x110·4) | code | fix-up |
| 0xbc | i32 | **wall count** | code + data | collision loop |
| 0xc0 | i32 | runtime render stamp (-1 in file) | code | `FUN_0041592e` |
| 0xc4, 0xc8 | 16.16 | bounding box min X, max X | code + data | `FUN_004d89c5` |
| 0xcc, 0xd0 | 16.16 | bounding box min Y, max Y | code + data | same |
| 0xd4, 0xd8 | 16.16 | bounding-box centre X, Y | data (validator) | |
| 0xdc..0xf0 | 16.16×6 | second box, only 12 regions; unknown | guess | |
| 0xf4, 0xfc | i32 | Object refs (rare) | code (fix-up) | |
| 0xf8 | i32 | Point ref (rare) | code (fix-up) | |
| 0x100 | i32 | Sound index (ambient/region sound) | code (fix-up) | |
| 0x104 | u16 | copy of the aux index | data | |
| 0x10c | i32 | Aux index | code (fix-up) | |
| 0x110 | i32 | **first index into RegionWallList** | code + data | fix-up |
| 0x114 | u16 | mostly 0xffff | | |

Region flag bits seen in code:
- 0x80000 / 0x100000 (byte 2, bits 8/0x10): region ignored by point-location and wall collision.
- 0x200: floor/ceiling texture origin is the first listed wall's p1, not the world origin.
- 0x04000000: no distance shading.

Validated: every listed wall has front or back equal to the region. Every wall appears in its front
region's list, and in its back region's list if it has one. The slices are contiguous. The bbox
matches the wall points.

**Point location** (`FUN_004d89c5`): cast a ray from (x, y) toward +X. Over all walls with
front ≠ back, find the nearest crossing with x' > x.
- If the wall runs toward −Y (y1 ≥ y > y2), the answer is its front region.
- If it runs toward +Y, the answer is its back region.
- The candidate must also have floor ≤ z < ceil and contain (x, y) in its bbox.

This reproduces the stored region of every placed object in all levels (validator).

**Collision** (`FUN_004a4a1f`) walks the current region's wall list. It prefilters by bbox ± radius
(obj+0x3c), then takes the perpendicular distance in the wall's frame (angle from atan2 of p2−p1).
- Solid walls (flag 0x2) block.
- Other walls let the object cross into the region on the other side, subject to floor/ceiling
  clearance (object height +0x34).
- An object whose new region is too low has its height clamped to (ceil − floor − 8).

## 5. Texs (100 B) + data

| off | type | meaning | conf |
|---|---|---|---|
| 0x00 | u32 | 2 = single static pic (1 entry, 1 frame, 1 rotation); 3 = sequence/rotations; bit 2 = loaded (runtime) | code + data |
| 0x04 | char[8] | name | data |
| 0x10 | u16 | frames (number of animation frames) | code `FUN_00488fae` (texture loader) |
| 0x12 | u16 | rotations (1, 8, 16, 32) | code |
| 0x14 | u16 | log2(rotations) | code |
| 0x16 | u16 | 0x800 >> log2rot (angle units per rotation step) | code + data |
| 0x18 | u16 | 11 − log2rot | code + data |
| 0x1a | u16 | n = number of sequence entries | code |
| 0x20 | i32 | set to -1 at load (runtime "last used frame stamp") | code |
| 0x24 | u16[16] | start entry index for action k (0 = idle; others e.g. walk/attack/die) | code `FUN_004a0c61` |
| 0x44, 0x48, 0x4c | ptr | runtime pointers to entries / pic table / mirror table | code |
| 0x50 | u32 | byte size of this tex's data = 12n + 5·frames·rot | code + data |
| 0x5c | u32 | own index | data |

Tex data, in Tex order:
1. `n × {u16 frame_base, u16 next, i32 delay(16.16 ticks), u32 runtime}`.
   - frame_base indexes the pic table: frame·rotations, to which the renderer adds the rotation.
   - next = following entry index; 98 = hold; 99 = end of sequence (sets the object's
     "animation finished" flag and continues with the next entry).
2. `frames·rot × i32` Pic indices (frame-major; -1 allowed).
3. `frames·rot × u8` mirror flags: 1 = draw the pic horizontally flipped, used to reuse rotations.

Some shipped sequences reference frames or next entries out of range in unused actions; ignore them.

## 6. Pics (40 B) — corrections to XRB.md

| off | meaning | conf |
|---|---|---|
| 0x0e | u16 **rows per column** (image height) | code + data |
| 0x10 | u16 **columns** (image width; renderer masks u with cols−1) | code + data |
| 0x12 | u16 sprite hotspot y (≈ height+2: bottom anchor) | data/guess |
| 0x14 | u16 hotspot x (≈ width/2) | data/guess |
| 0x16 | u16 1 | |
| 0x18 | u16 log2(column stride); column c starts at c << this | code `FUN_0041592e` |
| 0x1a | u16 own index | data |
| 0x20 | runtime pixel pointer | code |
| 0x24 | i32 XPG offset | code |

Pixels are column-major. In the XPG, columns are packed at `height` bytes each:
pic size = 0x0e × 0x10, verified by consecutive offsets of sprite pics. For walls and flats,
height = 2^0x18. Whether the loader re-pads sprite columns to the 2^n stride is not verified.

## 7. Objects (480 B)

| off | type | meaning | conf | evidence |
|---|---|---|---|---|
| 0x00 | u32 | flags. Movement/collision runs only if a bit in 0x30 is set; 0x20 = collides with other objects. Byte 2 bit 4 (0x40000) = **the player object**. | code | `FUN_004a4a1f`; fix-up sets `DAT_00510778` |
| 0x04 | u32 | runtime state bits | code | |
| 0x08 | i32 | own index (code reads it as u16) | data | |
| 0x0c, 0x10 | u32 | more flags | | |
| 0x14 | i32 | **Point** = position (X, Y and Z in the point) | code | fix-up, `FUN_004a4a1f` |
| 0x18 | i32 | second Point (home/spawn?), often -1 | code (fix-up) | |
| 0x24 | u16 | **angle** (2048/turn, 0 = +X, counter-clockwise) | code | `FUN_004a102a` |
| 0x26 | u16 | initial/rest angle (copied back to 0x24 by some effects) | code | |
| 0x2c | 16.16 | **z** (feet) | code + data | collision against floor (+0x0c) |
| 0x30 | 16.16 | previous/rest z | code | |
| 0x34 | 16.16 | **height** | code | head clearance |
| 0x38 | 16.16 | base height | data | |
| 0x3c | 16.16 | **radius** | code | object–object and wall collision |
| 0x40 | 16.16 | 9000/100/180/200 (draw distance? hit points?) | guess | compared to `DAT_006dfed8` |
| 0x48 | i32 | region index << 16 (mirror of 0x4c) | data | |
| 0x4c | i32 | saved region index (= 0xfc, or 0 if none) | data | |
| 0x54 | i32 | small int | | |
| 0x58 | char[8] | **object name** ("PLAYER", "DETH...", "BOT1") | data | |
| 0x64 | TC | **sprite**: tex @0x64, pic @0x6c | code | fix-up, `FUN_004a09fe` |
| 0x7c..0x98 | 16.16 | movement/AI parameters (200, 50, 25, 400, ...) | guess | |
| 0xac, 0xb0, 0xb4, 0xb8 | 16.16 | runtime velocity (z @0xb4; strafe @0xb0, forward @0xb8) | code | |
| 0xfc | i32 | **current Region** | code | fix-up, `FUN_004d85a1` |
| 0x100, 0x104 | i32 | Region refs. 0x100 is a secondary region: if obj+0x0c bit 0x20 is set, collision uses that region's +0x64 region. | code | `FUN_004a4a1f` |
| 0x108 | i32 | Wall ref | code (fix-up) | |
| 0x10c | i32 | Overlay ref | code (fix-up) | |
| 0x110 | i32 | Path ref | code (fix-up) | |
| 0x114 | i32 | Window ref | code (fix-up) | |
| 0x118..0x124 | i32 | small params (0x124 = 1000 often) | | |
| 0x140 | i32 | **effect index** (0 = OPLAY, 13 = OTHING, ...) | code (fix-up) + data | |
| 0x144, 0x148 | i32 | Object refs (owner/target) | code (fix-up) | |
| 0x14c | i32 | **next Object in the same region** | code + data | `FUN_004d85a1` |
| 0x150 | i32 | View ref (camera this object drives) | code (fix-up) | |
| 0x174 | i32 | Region ref (rare) | code (fix-up) | |
| 0x178, 0x17c | 16.16 | rare params | | |
| 0x18c..0x1a0 | i32×6 | Object refs (inventory/linked objects) | code (fix-up) | |
| 0x1a8 | i32 | Text ref | code (fix-up) | |
| 0x1ac | i16×2 | | | |
| 0x1b0 | i32 | Aux ref (sounds/messages) | code (fix-up) | |
| 0x1b4 | i32 | ObjectState ref | code (fix-up) | |
| 0x1bc, 0x1c0 | i32 | Point refs | code (fix-up) | |
| 0x1d0 | u32 | more flags (0x80000000, 0x1000000, ...) | | |

Player/start:
- **Objects[0] is the player** in every level: name "PLAYER", flag 0x40000, effect 0 (OPLAY),
  view 0.
- **hdr+0x4c** names an inert copy with the same point and angle (the start spot).
- The object named "NETPLAYR" is a template: it has a region index but is not in any region list.

Sprite rendering: pic = tex.pics[entry.frame_base + rotation], mirrored if the mirror flag is set.
- How the rotation index is derived from (view angle − object angle) was not traced. The expected
  form is ((rel + half step) & 0x7ff) >> tex+0x18. **guess**

## 8. Views (176 B)

| off | meaning | conf |
|---|---|---|
| 0x00 | type (3 = main, 2 = secondary such as TV/remote cameras) | data |
| 0x08 | own index | data |
| 0x10/0x12 | u16 width/height of the viewport (320×200, 128×128) | data + code (width used as the column count) |
| 0x1c | u16 width again | data |
| 0x2c | Object ref: camera object (0 = player for the main view) | code (fix-up, `FUN_004a0375`) |
| 0x30 | 16.16; 2.4 for the main view; FOV/zoom factor? | guess |
| 0x34 | runtime: eye z (copied from camera object +0x30) | code |
| 0x38 | 16.16 768: distance-shading divisor | code (`FUN_004187f8`, `FUN_0041592e`) |
| 0x3c | i16 @0x3e: angle offset added to the camera angle | code |
| 0x40 | 16.16 horizon row (99) | code |
| 0x5c | char[8] name ("TVVIEW", "REMOTE") | data |
| 0x68 | effect index (e.g. VMIRROR) | code (fix-up) |
| 0x80 | 16.16 200 | guess |
| 0x8c, 0x90 | 16.16 projection distance (≈ 220.2 for 320 px) | code (0x90 in `FUN_004187f8`) |
| 0x98, 0x9c | runtime cos/sin of the left-edge ray | code `FUN_004a0375` |
| 0xa0, 0xa4 | runtime per-column increments | code |
| 0xac | Aux ref | code (fix-up) |

Views do not hold the player start; see Objects[0] and hdr+0x4c.

## 9. Sounds (76 B)

| off | meaning | conf |
|---|---|---|
| 0x00 | flags (0, 0x21, 0x22, 0x62) | data |
| 0x08 | own index | data |
| 0x0c | char[24] sample name (e.g. "BACKSND", "FIRE01") | data |
| 0x24 | u16 0xffff | |
| 0x28 | small int 0..3 (priority/loop?) | guess |
| 0x30 | Sound ref (next in chain) | code (fix-up) |
| 0x34, 0x38 | runtime (zeroed by fix-up) | code |
| 0x44 | 0 or 0x8000 | guess |

## 10. Auxs (176 B)

An Aux is a bundle of sounds and message strings attached to walls, regions, objects and views.

| off | meaning | conf |
|---|---|---|
| 0x04 | 0/1 | |
| 0x08 | own index | data |
| 0x0e + 12k, k = 0..9 | ten char[12] slots. Slots 0–4 are sound names, resolved as the Sound refs below. Slots 5–9 hold free text (a message split over the slots, e.g. "ANOTHER DOOR HAS OPENED") or more names. | data |
| 0x88..0x98 | five i32 parameters (e.g. 0x9e, 0x20, 5) | guess |
| 0x9c..0xac | five Sound refs | code (fix-up) |

## 11. Overlays (72 B) — HUD/screen overlays

| off | meaning | conf |
|---|---|---|
| 0x08 | own index | data |
| 0x0c | TC (tex @0x0c, pic @0x14) | code (fix-up) |
| 0x24, 0x28 | source x, y inside the pic (copy origin) | code `FUN_0045675f` |
| 0x30 | draw order key | code (sorted in `FUN_0045675f`) |
| 0x3c | 10 | |

The screen position comes from the pic's own fields (+0x12/+0x14) for these screen pics. guess

## 12. Paths (452 B)

| off | meaning | conf |
|---|---|---|
| 0x08 | own index | data |
| 0x0c | char[] name (e.g. "RAFTPATH"; found by name, `FUN_00424a3c`) | code |
| 0x18 | node count | data |
| 0x20 | 16.16 speed (5, 10, 600) | guess |
| 0x44 | u16[64] node Point indices | data (RAFTPATH: points exist and are path points) |
| 0xc4 | u16[64] next Point for each node | data |

Unnamed paths carry other small values in these arrays; not understood.

## 13. ObjectStates (472 B)

One record per level, all zero in shipped data. The fix-up clears +0x1b4..+0x1bc and +0x1cc/+0x1d0
(runtime state).

## Open questions

- Wall +0x14/+0x18 extra points and +0x88 third region (about 100 and 12 walls); region +0xdc box.
- Exact meaning of wall flags 0x1/0x8/0x1000/0x2000 and region flags 0x80/0x10/0x20.
- Wall +0x9c default of 1.0: an x offset of 1 texel, or an editor artefact?
- The rest of the object parameter block (0x7c–0x98, 0x40) and the AI params.
- Rotation-index formula for sprites; sprite world scale (hotspot use) not traced.
- SampleTable, ObjTable512 and Globals6x64 semantics.
- Distance shading on/off (`DAT_00510820`) is a global, probably from INI renderflags.
