# gfx/*.PIC (`PC2`)
Source: GetPic = `FUN_00488146`. Header is 14 bytes, then `width*height` 8-bit pixels, **column-major** like XRB pics
(confirmed visually: the menu hand points sideways only when read column-major).

| Off | Type | Meaning |
|---|---|---|
| 0 | 4 | magic `PC2\0` |
| 4 | u16 | rows per column = height (16 in hand01) |
| 6 | u16 | columns = width (28) |
| 8 | u16 | ? (15) — likely x origin |
| 10 | u16 | ? (14) — likely y origin |
| 12 | u16 | ? (4) — flags |
hand01.pic: 14 + 16*28 = 462 bytes ✓
