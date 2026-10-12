# G.O.D.D. software renderer (lips.exe) — XRB / "orb" path

Clean-room notes: algorithms and constants restated from the decompilation / disassembly.
Function addresses are evidence pointers into `re/ghidra/decomp.c` and
`re/ghidra/decomp_extra.c` (the latter recovered by `re/scripts/FillGaps.java`; many renderer
callbacks live only there). Confidence: **H** = read directly from code and cross-checked,
**M** = read from code but not fully traced, **L** = inference.

The Quake BSP path (`LIPS00C.BSP`, FUN_00418a72 uses a second FOV constant 1.2361) is not
covered here.

---------------------------------------------------------------------------------------------

## 0. Numbers to remember

| Item | Value | Conf. |
|---|---|---|
| Fixed point | 16.16 everywhere (world coords, depths, screen y edges, texture coords) | H |
| Angle units | 2048 per circle, stored 16.16 (`0x8000000` = 360°) | H |
| Horizontal FOV | **410 angle units ≈ 72.07°** (`DAT_0056711c = 0x19a`) | H |
| Focal length | `F = (viewW/2) * 1.3764` px (1.3764 = cot 36°, double at `DAT_0056712c`); same value used vertically (aspect `DAT_005670d8` = 1.0) | H |
| tan(FOV/2) | `viewW/2 / F = 0.72654` (view+0x94) | H |
| Floor ray scale | `1/cos(36°) = 1.2361` (double at `DAT_00567124`) | H |
| Horizon row | `horizon = ((look + 127) * viewH) >> 8`; look clamped to [−512, +512] (`DAT_00567114/DAT_00567110`, OSETVIEW can change them) | H |
| Near plane | depth 8.0 world units (`0x80000`); endpoints closer than 6.0 are rejected/clamped | H |
| Texel size | 1 texel = 1 world unit on walls, floors, sprites and sky-height (no per-texture scale) | H |
| Light levels | 16 (0 = brightest … 15 = black), from PALETTE.PAL | H |
| Distance light | `row = baseLight + depth / visibility` (visibility = view+0x38, 16.16; 768.0 in all shipped levels; keys change it ±16 within [32, 1024]) | H |
| Draw order | front-to-back, a pixel is only written while it is still 0 ("hole filling"); colour 0 produced by a texel/colormap is replaced by index 8 | H |

---------------------------------------------------------------------------------------------

## 1. Views and the view window

Runtime View struct = the 176-byte record of the XRB `Views` section (loaded in place,
pointer table DAT_0056761c, current view DAT_005675a0, view being rendered DAT_00510990).
Offsets below were confirmed against LIPS00.XRB view 0 (see `re/scripts/xrb_sections.py`).

| Off | Type | Meaning | Conf. |
|---|---|---|---|
| +0x08 | i16 | view kind (0 = main view; non-zero views skip some effects) | M |
| +0x0c | i16 | window x origin in the render buffer | H |
| +0x0e | i16 | window y origin | H |
| +0x10 | i16 | width W | H |
| +0x12 | i16 | height H | H |
| +0x14 | ptr | pixel buffer | H |
| +0x18 | ptr | row-pointer table (H entries) | H |
| +0x1c | i32 | pitch | H |
| +0x24/+0x28 | ptr | "LightBS" buffer + rows (dynamic light buffer, optional) | M |
| +0x2c | ptr | object the view is attached to (the player) | H |
| +0x34 | 16.16 | **eye Z** (absolute) — copied from object+0x30 every frame (FUN_004a0375) | H |
| +0x38 | 16.16 | **visibility** (fade distance per light level) | H |
| +0x3c | i16 | shade-table index (always 0) | H |
| +0x3e | i16 | view yaw offset relative to the object ("VAangle"; total yaw = object angle + this) | H |
| +0x40 | i16 | look (y-shear) value, clamped [−512, 512] | H |
| +0x42 | i16 | horizon row (derived) | H |
| +0x70 | 16.16 | sky texels per screen column | H |
| +0x74 | 16.16 | sky texels per screen row = skyColumnLength / (2H) | H |
| +0x7c/+0x7e | i16 | sky fill colours (first/last texel of a sky column) | H |
| +0x80 | 16.16 | H (used by horizon and sky) | H |
| +0x84/+0x88 | 16.16 | bufferW/W, bufferH/H | M |
| +0x8c | 16.16 | focal length F | H |
| +0x90 | 16.16 | vertical focal = F * aspect(1.0) | H |
| +0x94 | 16.16 | (W/2)/F = tan 36° | H |
| +0x98/+0x9c | 16.16 | cos/sin of the left frustum edge angle | H |
| +0xa0/+0xa4 | 16.16 | per-column delta of that vector (chord to the right edge) / W | H |

### SetViewSize (FUN_004ac081)
Given W, H (and optionally an external buffer): (re)allocates the buffer and row table, then
* F = (W/2) * 1.3764 (computed as `(W>>1) * round(1.3764*65536)` in 16.16), vertical focal =
  F * 1.0, +0x94 = (W/2)/F;
* +0x80 = H; sky steps (+0x70, +0x74, fill colours), +0x84/+0x88 scale factors.
So the **horizontal FOV is fixed at 72°**; changing H only changes the vertical extent.

### Horizon (FUN_004ac49f)
`set_look(view, yawOffset, look)`: stores yaw offset at +0x3e, clamps look to
[DAT_00567114, DAT_00567110] = [−512, 512], stores at +0x40, and sets
`horizon = ((look + 127) * H) >> 8` (if H is negative it uses 0). With look = 0 the horizon is
row `127*H/256` (one row above centre for H=200 → 99). Look-up/down is therefore **pure
y-shearing**, no true pitch; ±512 shifts the horizon by ±2H. Also copies look into the player
object (+0x28). Keys/mouse that change `look` are in physics.md.

### Window size keys (action table at 0x51310f; handlers in decomp_extra.c)
Only active when there is exactly one view (DAT_00567610 == 1). Buffer size is
DAT_005670d4 x DAT_005670d6 (320x200 by default, 640x480 if `[startup] graphicsmode=640x480`).

| Action | Handler | Effect |
|---|---|---|
| incviewwidth | FUN_00478858 | W += 8 (max buffer width), x0 = (bufW − W)/2 |
| decviewwidth | FUN_00478912 | W −= 8 (min bufW/2), recentred |
| incviewheight | FUN_004789d0 | H += 6 (max bufH), y0 = (bufH − H)/2 |
| decviewheight | FUN_00478a8a | H −= 6 (min bufH/2), recentred |
| setfullviewsize | FUN_004787c9 | origin (0,0), W×H = buffer |
| incviewvisibility | FUN_00478b48 | visibility += 16.0 (max 1024.0) → lighter (keypad `*` per keyboard.txt, M) |
| decviewvisibility | FUN_00478b9a | visibility −= 16.0 (min 32.0) → darker (keypad `/`, M) |

Each size change calls SetViewSize then re-applies the horizon with the current look.
(The 472x328 window seen in screenshots is not produced by these ±8/±6 steps from 640x480;
it presumably comes from window/overlay layout data — open question.)

---------------------------------------------------------------------------------------------

## 2. Frame setup and camera

RenderView = FUN_004ab15f, per-frame init = FUN_004ab9b4, camera = FUN_004ab912,
per-view trig = FUN_004a0375.

1. Camera position = position of the view's object (`object+0x14` → Point; X,Y,Z at
   Point+0x14/+0x18/+0x1c). Eye Z = view+0x34 (object+0x30 = floor Z + eye height; see physics).
2. Camera yaw θ = object angle (i16 at object+0x24) + view+0x3e. The engine stores
   `c = cos(−θ)`, `s = sin(−θ)` (θ forced to 1 if 0).
3. Left frustum edge angle `a0 = θ + 205` (half FOV); view+0x98/0x9c = cos/sin of `−a0`, and the
   step to `−(a0 − 410)` divided by W (used by floors and sky).
4. Frame counter DAT_0056754c increments; points/regions/walls are lazily transformed once per
   frame (stamp at Point+0x50, Region+0xc0, Wall+0xd4).

### Point transform (FUN_0049b1fe)
For world point P, with d = P − cam:
* depth `vx = dx*cosθ + dy*sinθ` (Point+0x20), lateral `vy = −dx*sinθ + dy*cosθ`
  (Point+0x24, positive = to the left), dz = Pz − camZ (Point+0x28).
* frustum distances: `eL = vy − vx*tan36` (Point+0x2c), `eR = vy + vx*tan36` (Point+0x30)
  — a point is inside the horizontal frustum when eL ≤ 0 ≤ eR.
* view-relative bearing `atan2(vy, vx)` (Point+0x34) and cheap ground distance (Point+0x38).
* screen x (Point+0x0c) = `W/2 − vy*F/vx` (rounded, float math; separate branches for points
  behind the camera).
* scale `s = F/vx` (or ±1 when |vx| ≤ 10), screen y (Point+0x10) =
  `horizon + s*(eyeZ − Pz)`.
* flag bit 0 at Point+4 set when the point lands inside the buffer.

Screen coordinates: x grows right, y grows down. Projection of a world height z at depth vx:
`y = horizon + (eyeZ − z) * F / vx`.

---------------------------------------------------------------------------------------------

## 3. Visibility: region traversal, draw list, clipping

Levels format: Regions = sectors (308 B), Walls = linedefs (240 B), Points (100 B).

### Traversal (FUN_004ab15f → FUN_00498009 → FUN_004995e5)
* A region queue (max 500, DAT_006c4ef8) starts with the camera's region (object+0xfc).
  Each region is processed once per frame (stamp Region+0xc0). Processing a region
  (FUN_00498009): loop its wall list (Region+0xb8, count +0xbc) three times — one-sided walls
  first, then two-sided (portal) walls, then special walls (flag byte2&8, polyobject/"cw" walls)
  — and then its objects (linked list Region+0xb4, next = object+0x14c). The queue is walked in
  order while it grows, i.e. breadth-first. (H)
* Per wall (FUN_004995e5): transform both endpoints; **back-face test** by bearing order: let
  d = bearing(p1) − bearing(p2) mod 360°; if d > 180° the wall is seen from its back side. Back
  sides of one-sided walls are skipped; two-sided walls swap endpoints/regions and set a "back"
  flag. Net effect: the front side is on the **right** of p1→p2 (Doom convention). (H)
* Near-plane / frustum clip in the frustum-distance space: the visible span is where the wall
  segment crosses the region between the two frustum edges; depth at a clipped end is
  interpolated with `t = eL1/(eL1 − eL2)` and clamped to ≥ 8.0. Columns [xL, xR) are clipped
  to [0, W) and tested against the current open column range. (H)
* A **portal** (two-sided wall whose back region has a ceiling above the eye or is otherwise
  open) appends the neighbour region to the queue. (H)
* Visible walls/objects become "VDraw" records (0xc0 bytes, pool of 2400, DAT_00567578) kept in
  a list sorted by left column (FUN_0049bd64).

### Draw order and clipping
* FUN_004acd94 picks the next VDraw to draw among column-overlapping ones by comparing their
  depths (depth at both ends +0x40/+0x44, interpolated at overlaps) — nearest first. (M — the
  comparison logic is long; the observable result is front-to-back.)
* Per-column vertical clip buffer ("LevelClipBuffer", 640 × {i16 top, i16 bottom}) holds the
  still-open interval of each column; walls shrink it. Floors/ceilings use it for span extents.
* All primitive drawers **only write pixels that are still 0** in the frame buffer; the buffer
  is zeroed after each present (FUN_0046d9c9 / FUN_004db8e8). A texel whose colormapped value is
  0 is written as **8** (near black) so it counts as filled. (H, disassembly of 0x401010,
  0x4048e4)
* "Render levels" (up to 120, 0x1a4-byte records at DAT_00558e74) are additional passes for
  regions stacked above/below another (Region+0x5c / +0x60): a floor or ceiling of a region with
  a stacked neighbour opens a new level that is rendered with its own clip buffer
  (FUN_00417d58/FUN_00417f99). Heights in such passes are wrapped modulo 512.0 relative to the
  eye (FUN_0041b84e) — room-over-room support. (M)

---------------------------------------------------------------------------------------------

## 4. Walls

Relevant Wall fields (runtime = XRB record): +0x00 flags, +0x0c/+0x10 point p1/p2 (indices in
the file), +0x2c/+0x44/+0x5c texture slots (0x18 bytes each; pic pointer at slot+8 →
+0x34 upper, +0x4c middle, +0x64 lower), +0x80 front region, +0x84 back region (0 = one
sided), +0x8c length (16.16, integer isqrt of the integer dx²+dy²), +0x9c **(TexX+1)** 16.16,
+0xa0 TexY 16.16, +0xa4 i16 light (current), +0xa8 i16 light (base copy), +0xaa i16 fade.
(ORB loader FUN_0047cb09 and renderer; H)

### Setup — one-sided (FUN_0041b903)
* Scale at both ends `sL = vF/depthL`, `sR = vF/depthR`; **scale is interpolated linearly in
  screen x** (exact, since 1/z is affine in screen space).
* Top edge `yTop(x) = horizon + s(x)*(eyeZ − ceilZ)`, bottom edge
  `yBot(x) = horizon + s(x)*(eyeZ − floorZ)` (front region ceiling +0x10, floor +0x0c),
  both as 16.16 start + per-column step.

### Setup — two-sided (FUN_00414a47)
Front region (ceil Cf, floor Ff), back region (Cb, Fb). Edges: front ceiling line, front floor
line, `min(Cf, Cb)` line (upper wall bottom), `max(Ff, Fb)` line (lower wall top). Upper part
drawn between Cf and min(C), lower part between max(F) and Ff, middle (optional masked
texture) between them.

### Column loop (FUN_0041bc9d solid, FUN_0041592e two-sided)
For every column x in [xL, xR):
* **u (horizontal texel)** — exact perspective: the two endpoint frustum distances are linear
  in x (`e1(x)`, `e2(x)`, VDraw+0x28..+0x34); the ray through column x hits the wall at
  `t = e1/(e1 − e2)`; `u = floor(t * length + TexX)` (the +1 stored in +0x9c is removed again).
  Texture column = `u & (numColumns − 1)`.
* **v (vertical texel)**: `step = (Cf − Ff) / (yBot − yTop)` world units per pixel (always the
  full front wall span), `v(y) = (y − yAnchor)*step + TexY`, then `& (columnLength − 1)` in the
  drawer, i.e. textures tile vertically. Anchors:
  * one-sided wall: top of texture at the **ceiling** (yAnchor = yTop);
  * upper part of a two-sided wall: texture **bottom aligned to the lower ceiling** (v at the
    front ceiling = TexY + (Cb − Cf) − columnLength);
  * lower part: texture top at the **higher floor** (max(Ff, Fb));
  * middle (masked) part: texture top at the lower ceiling (M).
* Light per column: see §7 (depth interpolated linearly from depthL to depthR).
* A wall slot with **no texture** draws the sky in that column range instead (callback
  FUN_004181da → FUN_0041ac87).
* Floor and ceiling visplanes for the column are emitted through the region's plane callbacks
  (see §5) for the open interval above yTop / below yBot.

### Texture storage
Pic record fields (40 B, runtime = XRB): +0x0e u16 **column length** (file "width"),
+0x10 u16 **number of columns** (file "height"), +0x12 u16 y-origin (sprites), +0x14 u16
x-origin (sprites), +0x18 u16 log2(column length) ("shift"), +0x20 pixel pointer.
Pixels are **column-major**: column c starts at `pixels + c * columnLength` (walls/floors use
`c << shift`). This is why wall pictures look transposed when dumped as rows. Wall textures
need power-of-two column length and column count (both masked).

### Column drawer (job type 5/0xd, asm at 0x4048e4)
For each pixel down the column: skip if dest ≠ 0; `texel = column[(v>>16) & mask]`;
`c = colormap[texel]`; write `c ? c : 8`; `v += step`.

---------------------------------------------------------------------------------------------

## 5. Floors and ceilings ("Flines")

Classic span builder (Doom R_MakeSpans style): one Fline record (0x3c bytes) per screen row;
FUN_00418c27 compares the previous column's open [top, bottom) for the plane with the current
one, closing spans (FUN_00418643 → job type 1 floor / 10 ceiling) and opening rows
(FUN_004187f8). Error "CheckCanDrawFlineSpan: out of segs for row %d" comes from the span
allocator.

Row setup (FUN_004187f8) for screen row y:
* `dy = y − horizon` (1 if 0). Rows above the horizon use the ceiling height (Region+0x10),
  below use the floor (Region+0x0c).
* `dist = (eyeZ − planeZ) / dy * vF` (forward depth of the row).
* World position of the left screen edge: `X0 = camX + dist*1.2361*cos(−a0)`,
  `Y0' = −camY + dist*1.2361*sin(−a0)` ; per-pixel step `dist*1.2361 * (+0xa0, +0xa4)`.
  (1.2361 = 1/cos 36° turns forward depth into distance along the edge ray.)
* Texture offsets: floor +0x4c/+0x50, ceiling +0x54/+0x58 (16.16) added to X/Y'. If the region
  has flag byte1&2 the texture origin is relative to a reference point (moving/rotating floor).
* **Texture index = ((Y' >> 16) mod N) * N + ((X >> 16) mod N)**, N = 2^shift (square,
  power of two, 1 texel per world unit). Y is negated, so texture rows run southward.
  (packed 16.16 coordinates decoded by `shld` in the asm at 0x401010.)
* Light: region floor light i16 @+0x82 / ceiling light @+0x84, plus `dist/visibility`
  (unless region flag byte3&4 = "no distance fade"). Fade: floor @+0x8c, ceiling @+0x8e.
* Span drawer: same "write only if 0, 0→8" rule as walls.

A region plane with **no texture** shows the sky (plane callback FUN_004181da).

---------------------------------------------------------------------------------------------

## 6. Sky (FUN_0041ac87, column setup in FUN_004ab9b4)

Zone block (last 0x7c bytes of the XRB, copied to DAT_005676ac): sky picture index
(DAT_005676fc), sky angular span in angle units (i16 at DAT_00567714: 2048 = 360° in LIPS00..17,
409 ≈ one screen in LIPSTART), texels per angle unit (DAT_00567718: 0.3125 → 640 columns).
* Horizontal: starting column for x = 0 is `((−θ − 205) mod span) * texelsPerUnit`, advancing
  `(410/W) * texelsPerUnit` per screen column (linear in angle, not atan-corrected), wrapped by
  the picture's column count. The sky scrolls with yaw only (no parallax).
* Vertical: sky row for screen y = `(H + (y − horizon)) * columnLength / (2H)` — the middle of
  the sky column sits on the horizon and the column spans 2H rows; above/below the picture the
  first/last texel colour is repeated (2048-byte padded copy of the column).
* Drawn with the unlit shade row 0 (no distance fade) through the column drawer.

---------------------------------------------------------------------------------------------

## 7. Lighting and palette use

PALETTE.PAL = 16 shade rows + 256x256 50% blend table (re/formats/PAL.md).

For walls (+0xa4), floors (+0x82), ceilings (+0x84) and objects (+0x44):
```
L = baseLight
if distance-fade enabled (DAT_00510820, default 1) and the surface's "no-fade" flag is clear:
    L += depth / visibility            # 16.16 divide, depth = forward distance
if fullbright powerup (DAT_0051081c): L = DAT_0054868c
if clamp mode (DAT_00510818): L = clamp(L, DAT_0054868c, DAT_00548684)
L = clamp(L, 0, 15)                     # objects: clamp to 0..14
colormap = fade == 0 ? shade[L] : blend[(fade + L) * 256 ...]
```
* Light values are stored as **darkness** (0 = full bright). Shipped levels: walls mostly 8,
  region floor 4 / ceiling 8 (LIPS00), 8/12 (LIPS05), 6/6 (LIPSTART). Editor lights are negated
  and clamped by `floorlightmin`/`ceilinglightmin`/`walllightmin`/`objectlightmin`
  (default −12 → max darkness 12) when an ORB is converted (FUN_0047cb09). (H)
* `visibility` (view+0x38) = 768.0 in every shipped XRB view; OSETVVIS/OSETVIEW effects and the
  inc/decviewvisibility keys change it (range 32..1024). So a surface gets one level darker per
  768 world units of depth by default. The INI/keyboard "fade factor" is this value. (H)
* Wall light is linearly interpolated across columns via depth; floor light is per row.
* Flicker objects (object byte+0xf & 0x10): L += 2*(rand & 7), clamped to the object's limits,
  values > 15 reflect as 31 − L. (M)
* `fade` ≠ 0 mixes the texel 50/50 with palette colour `fade + L` (blend table row), giving
  coloured fog ramps; all shipped regions have fade 0. (M)
* An optional per-pixel "LightBS" buffer (DAT_00515bac path) exists for dynamic light; not used
  by default. (L)

---------------------------------------------------------------------------------------------

## 8. Objects / sprites

Setup FUN_0049a698, frame choice FUN_0049a558, extents FUN_004195ec, draw FUN_00419970.

* Object origin is transformed like a point; skipped if depth < 8.0.
* **Rotation frame**: the object's texture (Texs record via object+0x64) has
  `nrot` = u16@0x12 rotations, sector = u16@0x16 (= 2048/nrot), shift = u16@0x18 = log2(sector),
  and frames×rotations picture table (+0x48) and per-entry mirror flags (+0x4c).
  ```
  rel = objAngle − (camYaw + bearingOfObjectInView) + 1024      # = objAngle − bearing(obj→cam)
  rot = ((rel + sector/2) >> shift) & (nrot − 1)
  pic = table[animFrameBase + rot]; mirror = flags[animFrameBase + rot]
  ```
  rot 0 = object facing the camera; increasing index = object turned counter-clockwise from
  that. (H)
* Size/position: 1 texel = 1 world unit scaled by `F/depth`. Horizontal extent: x-origin
  (pic+0x14) texels left of the object's position and (numColumns − x-origin) to the right
  (swapped when mirrored). Vertical: top at world `objZ + yOrigin` (pic+0x12, objZ =
  object+0x2c), bottom at `objZ + yOrigin − columnLength`. (H)
* Columns are drawn through the per-column clip interval of the current render level; texel 0
  is transparent; writes only into still-empty pixels (job type 3, asm 0x402e8d). Mirrored
  sprites read column (numColumns−1−c).
* Light: object light i16 @+0x44 + depth/visibility (unless object flag byte3&4), clamp 0..14;
  fade @+0x50.
* **Translucent** objects (object byte+1 & 1) use the blend table: `dst = blend[texel][dst]`
  (job type 4, asm 0x4041ae; texel 0 leaves dst unchanged). These, masked two-sided middle
  textures and "cw" walls go through an off-screen XBuffer and are composited later
  (FUN_0041f431 "DrawXBufferBlendPatch"), because the main pass is front-to-back. (M)
* Polygon objects (object+0x1d2 & 8) use a separate polygon renderer (FUN_0049ab8e,
  "DrawPolyObjectInGodd"); not analysed.

---------------------------------------------------------------------------------------------

## 9. Draw-list entry types (FUN_004ab15f dispatch)

| VDraw flags | Function | Meaning |
|---|---|---|
| byte5 & 8 | FUN_0041f431 | XBuffer blend patch (translucent composite) |
| byte5 & 2 | FUN_00419483 | debug/map line |
| byte4 & 1 | FUN_00419970 | sprite object |
| byte4 & 0x10 and & 2 | FUN_0041d857 | special wall (polyobject / multi-point wall) |
| byte4 & 4 | FUN_0041592e | two-sided wall (upper/lower/masked middle) |
| otherwise | FUN_0041bc9d | one-sided wall |

Primitive jobs (FUN_004ac9f5, type at job+0x1a): 1/10 floor/ceiling span, 3 masked column,
4 translucent column, 5/0xd textured wall column, 6/7/0xe/0xf variants (light-buffer,
XBuffer), 0xb visplane hook.

---------------------------------------------------------------------------------------------

## 10. Struct cheat-sheet (runtime == XRB record unless noted)

**Point (100 B)**: +0x0c screen x, +0x10 screen y, +0x14/+0x18/+0x1c world X/Y/Z,
+0x20 depth, +0x24 lateral, +0x28 dz, +0x2c/+0x30 frustum distances, +0x34 bearing,
+0x38 distance, +0x50 frame stamp.

**Region (308 B)**: +0x0c floor Z, +0x10 ceiling Z, +0x4c..+0x58 floor/ceiling texture offsets,
+0x5c/+0x60 stacked region above/below, +0x82 floor light, +0x84 ceiling light, +0x88/+0x8a
base copies, +0x8c floor fade, +0x8e ceiling fade, +0xb4 object list, +0xb8 wall pointer list,
+0xbc wall count, +0xc0 frame stamp. Texture slots +0x1c (floor) / +0x34 (ceiling) with pic
pointers at +0x24 / +0x3c (null pic → sky).

**Wall (240 B)**: see §4.

**Texs (100 B)** animated texture: u16 +0x10 frames, +0x12 rotations, +0x14 log2(rotations),
+0x16 sector size, +0x18 log2(sector), +0x1a entry count; runtime tables +0x44 (frame table,
12 B each), +0x48 (picture pointer per frame×rotation), +0x4c (mirror byte per entry).

---------------------------------------------------------------------------------------------

## 11. Open questions

1. Exact placement of the 3D window in the 640x480 HUD layout (472x328 at 80,60): which data sets
   it (Windows/Overlays sections?) and whether the view buffer is scaled when blitted
   (view+0x84/+0x88 hint at scaling).
2. Fine details of the VDraw depth-ordering comparator (FUN_004acd94) — needed only to
   reproduce rare overlap artefacts.
3. Masked/translucent compositing order (XBuffer path) not fully traced.
4. `fade` blend-row semantics are inferred from code; no shipped region uses it.
5. Stacked-region ("render level") passes and the modulo-512 height wrap are only partially
   understood.
6. Whether the main-view clear happens before every frame in all display modes (two clear sites
   found).
