# .XRB — compiled orb (level) + .XPG — pixel blob

Source: `ReadCompiledOrb` = `FUN_004d5968` (lips.exe). Validated by `re/scripts/xrb_probe.py`
on LIPSTART and LIPS00; pics render correctly (`re/scripts/pics_sheet.py`).

## File layout
| Offset | Size | Meaning |
|---|---|---|
| 0x00 | 4 | magic `O11\0` |
| 0x04 | 0x1c | source path, e.g. `D:\GODD\LIPS00.ORB` (NUL padded) |
| 0x34 | 4 | u32 size of the matching `.XPG` (exact file size) |
| 0x5c | 4 | u32 build stamp; loader rejects `< 0x07059901`. Shipped: `0x12089902` (= 12-08-99 #02) |
| 0x40..0x64 | | other globals copied out (start object idx @0x4c, view/state values @0x50..0x60) — TODO |
| 0x68 | rest | body |

## Body
`body[0..300]` = 15 section descriptors, 20 bytes each, in this order:
Texs, Pics, Points, Paths, Regions, Walls, Objects, ObjectStates, Views, Auxs, Sounds, Texts, Windows, Overlays, ActionNodes.

Descriptor = `u32 max, u32 used, u32 allocated, u32 record_size, u32 stale_pointer`.
Record arrays hold `allocated` records; only the first `used` are live.

| Section | rec size | parser |
|---|---|---|
| Texs | 100 | FUN_004d4b42 |
| Pics | 40 | FUN_004d4c59 |
| Points | 100 | FUN_004d4cfe |
| Paths | 452 | FUN_004d508a |
| Regions | 308 | FUN_004d5105 |
| Walls | 240 | FUN_004d5180 |
| Objects | 480 | FUN_004d51fb / FUN_004d5276 |
| ObjectStates | 472 | FUN_004d52f1 |
| Views | 176 | FUN_004d53aa |
| Auxs | 176 | FUN_004d53f9 |
| Sounds | 76 | FUN_004d5474 |
| Texts | 108 | FUN_004d54ef |
| Windows | 112 | FUN_004d5564 (only if hdr field 0x40 > 0) |
| Overlays | 72 | FUN_004d55d7.. |
| ActionNodes | 116 | FUN_004d56ed.. |
**Correction:** sections are NOT contiguous in descriptor order. The loader inserts extra blocks:
0x180 + 0x800 bytes after ObjectStates, hdr[0x40]×0x3c bytes before Texts, per-text data, and then
the region wall list (hdr[0x24] bytes) plus a 0x7c-byte tail at the end.
See `XRB-records.md` §1 and `re/scripts/xrb_lib.py`; the computed layout matches the file size for all 19 files.
Windows records are 112 B (not the 0x3c-byte hdr[0x40] block). Field layouts are also in `XRB-records.md`.

Every record starts with `u32 type` then an 8-char NUL-padded name at +4.

### Texs (100 B) — animated/composite texture definitions
Records are followed by a variable data area. For each live tex:
- `n = u16 @0x1a`, `w = u16 @0x10`, `shift = u16 @0x14`, `size = u32 @0x50`
- data = `n*12` bytes (frame/entry table) + `(w<<shift)*4` + `(w<<shift)` bytes; `size == n*12 + 5*(w<<shift)` holds for every tex.
- runtime pointers written at +0x44/+0x48/+0x4c.

### Pics (40 B) — images in the .XPG
- `u16 @0x0e` = rows per column (height), `u16 @0x10` = columns (width): see XRB-records.md §6, more u16 @0x12, 0x14, 0x18 (same as PC2 header fields).
- `i32 xpg_offset @0x24` (-1 = not in XPG). Pixels are 8-bit palette indices, `width*height` bytes.
- Wall textures appear stored **column-major** (text is transposed) — confirm in the renderer.
- Pic 0..2 are `DEF_Flor`, `DEF_Ceil`, `DEF_Wall` (64x64).
