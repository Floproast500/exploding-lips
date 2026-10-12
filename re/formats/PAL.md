# PALETTE.PAL (`PL3`)

Status: **confirmed** from the loader (LoadPalette = FUN_00469ed5) and the renderer.

| Offset | Size  | Meaning |
|--------|-------|---------|
| 0x000  | 4     | magic `PL3\0` |
| 0x004  | 2     | u16 `ntables` = 1 (number of shade tables; global DAT_00567160) |
| 0x006  | 2     | u16 `nlevels` = 16 (light levels per shade table; global DAT_00567162) |
| 0x008  | 768   | 256 x RGB, **6-bit VGA** (0..63; scale by 255/63) |
| 0x308  | 32    | byte table (DAT_00567464, all zero in the shipped file; only used to build a 16-bit value for the sky/poly path) |
| 0x328  | ntables*nlevels*256 | **shade tables**: table t, light level L (0 = full bright .. 15 = black) is a 256-byte remap at `0x328 + (t*16 + L)*256` |
| 0x1328 | 65536 | **translucency table** `blend[a*256 + b]`: palette index closest to the 50/50 mix of colours a and b. Symmetric; index 0 acts as "transparent": `blend[0][x] = blend[x][0] = x`. |

Total 70440 = 8 + 768 + 32 + 16*256 + 65536.

The old "17 blocks x 16 shade levels" hypothesis is **refuted**: there is one 16-level shade
table followed by a 256x256 50% blend table (so "row 16" is blend row 0, which is the identity).
Measured mean luminance of shade rows: L=0 109.5, 1 104.2, 2 96.1, ... 8 55.2 ... 14 14.2,
15 0.0 → approx. `colour * (16-L)/16`, with row 15 forced to black.

How the renderer uses them (see re/engine/renderer.md "Lighting"):
* normal surfaces: `colormap = shade[t][clamp(light + depth/visibility, 0, 15)]`
  (t = view field +0x3c, always 0 here).
* surfaces with a non-zero "fade" value f: `colormap = blend row (f + light)`, i.e. the texel
  is mixed 50/50 with palette colour `f + light` (fog/tint ramps).
* translucent sprites/walls: `dst = blend[texel*256 + dst]`.

`GODDPOLY.PAL` (8962 bytes): no magic; looks like the same 6-bit palette at offset 4. TODO.

# gfx/*.PIC — see PIC.md

# *.XPG — see XRB.md
