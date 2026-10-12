# MATH.DAT — trig tables

Status: **confirmed** (loader FUN_00480b8f, users FUN_00468c7f/FUN_00468cc2/FUN_00468d05,
validated by `re/scripts/math_dat.py`).

20480 bytes, three little-endian i32 arrays, no header:

| Offset | Count | Global (runtime) | Contents |
|--------|-------|------------------|----------|
| 0x0000 | 2048 | DAT_0056757c "SinTable" | `trunc(65536 * sin(i * 2π/2048))` |
| 0x2000 | 2048 | DAT_0056756c "CosTable" | `trunc(65536 * cos(i * 2π/2048))` |
| 0x4000 | 1024 | DAT_00567584 "ITanTable" | arctangent: `atan((i-512)/512)` expressed as a 16.16 angle in 2048-units-per-circle, i.e. `trunc(atan((i-512)/512) * (1024/π) * 65536)`. i=0 → −45° (−0x1000000 + 1), i=512 → 0. |

The sin/cos tables match truncation-toward-zero exactly for all 2048 entries; the ITan table
matches truncation for 1021/1024 entries (the rest differ by 1 LSB — float rounding in the
original generator). A reimplementation may regenerate them with these formulas or just load
the file.

## Angle convention (engine-wide)

* Full circle = **2048 units**. Angles are usually stored as **16.16 fixed point** of those
  units (e.g. 90° = 512 = `0x2000000`, 360° = `0x8000000`); trig lookups use the integer part
  masked with `& 0x7ff`.
* Degrees from INI/editor are converted with `units = deg * 512 / 90` (e.g. `playerspeedmaxt`,
  sky span, object angles in the ORB loader).
* World axes: X east, Y north (standard math orientation), angles counter-clockwise from +X.
  (Floor textures negate Y, see renderer.md.)

## Helpers built on the tables

| Function | Meaning |
|---|---|
| FUN_00468c7f(a) | `sin[a & 0x7ff]` (argument = integer angle units) |
| FUN_00468cc2(a) | `cos[a & 0x7ff]` |
| FUN_00468d05(x, y) | `atan2(y, x)` → 16.16 angle in `[0, 0x8000000)`. Octant reduction: ratio r = smaller/larger (16.16), table index = `(r*512 + 512.5) >> 0` clamped to 1023, then 0°/90°/180°/270° offsets with sign flips; returns 0 if both are 0. |
| FUN_00468f3e(x1,y1,x2,y2) | cheap distance: `max(|dx|,|dy|) + min(|dx|,|dy|)/2` |
| FUN_00468fac(n) | integer square root (used for wall lengths) |
