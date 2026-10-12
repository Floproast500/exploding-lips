# .GAM save file and GODDRUN.DAT preferences

All integers little-endian. Offsets are *logical* (in-memory) offsets – see "text mode" below.

## Text-mode I/O caveat (applies to both files) **[H]**
Both files are opened by the Watcom C runtime **without `b`**: save written with `"w"`, read
with `"r+"`; GODDRUN.DAT written with `"w"`, read with `"r"` (strings @0x523652 `w`,
@0x52377C `r+`, @0x523606 `r`). In text mode every 0x0A byte is written as 0x0D 0x0A and
CR LF is folded back on reading. Evidence: GODDRUN.DAT/goddrun.tad are 1429 bytes although the
struct is 0x594 = 1428 bytes; the only CR LF is at file offset 0x380 where the logical field
value is 10 (0x0A). Everything after it in the file is shifted by +1.
A reimplementation must (a) when reading, collapse every CR LF to LF before interpreting,
(b) when writing a byte-compatible file, expand 0x0A to 0x0D 0x0A. (A 0x1A byte may also be
treated as EOF by the Watcom text reader – unverified.) LIPSTART.GAM happens to contain no
0x0A so its size equals the logical size 0x1CF8.

## LIPSTART.GAM (7416 = 0x1CF8 bytes)

Code: LoadSaveGameFile FUN_004bb89c, WriteSaveGameFile FUN_004bbc10,
PlayerPositionToSGD FUN_004bbe1a, SGDToPlayerPos FUN_004bcfc6 / FUN_004bcdc7,
slot pointer FUN_004bbafa, free slot FUN_004bbd3d, GAME* effects FUN_004bc2b8.. (script.md §8.3).
File name = `<startup orb>.GAM` in the game directory (log: `LoadSaveGameFile …\LIPSTART.GAM`).

* 6 records × 0x4D4 (1236) bytes. Record 0 is a header/scratch record, records 1–5 are the five
  character slots shown in the main menu.
* Loading: the whole file is read (if larger than 0x1CF8 the larger size is used; slot count =
  size/0x4D4). For slots 1–5 whose index field is not 1..5 the record is zeroed and its index
  set. The game always writes exactly 0x1CF8 logical bytes. Saving to a CD install is refused.
* Log line per slot: `--- idx=%d, name=%s, orb=%s, states= L=%d MA=%d A=%d MO=%d E=%d TF=%d D=%d EK=%d`
  printing name, orb and states 1,2,3,4,5,10,11,12.

### Slot record (0x4D4 bytes)
| off | size | field | evidence / LIPSTART.GAM slot 1 |
|---|---|---|---|
| 0x000 | u32 | flags: bit0 active, bit1 created by GAMENEW, bit2 saved by GAMESAVE (GAMEGOTO uses the stored orb only if bit2 set) | 3 |
| 0x004 | u32 | slot index 1..5 | 1 |
| 0x008 | u32 | player object word 0 (type/flags) | 0x01040030 (= PLAYER object type in XRB) |
| 0x00C | u32 | player object word 1 (flags) | 0x02000000 |
| 0x010 | u32 | player object word 3 | 0 |
| 0x014 | u32 | player controller +0x0C | 0 |
| 0x018 | char[20] | character name (NUL padded) | `rew` |
| 0x02C | 4 | (unused, part of name area 0x18..0x2F) | 0 |
| 0x030 | char[24] | orb name at save time (`DAT_0056e8e0+0x27C`, first 24 chars) | `lipstart` |
| 0x048 | i32×3 | position x, y, z (16.16 fixed) | 0x3CD477CC, 0x420F8752, 0 |
| 0x054 | i32×4 | player object words 0x2B..0x2E (velocity / motion) | 0 |
| 0x064 | i32 | object word 0x0B | 0 |
| 0x068 | i32 | object word 0x0C (16.16; 48.0 = height?) | 0x00300000 |
| 0x06C | i32 | object word 0x0D (16.16; 48.0) | 0x00300000 |
| 0x070 | i32 | domain/sector index the player stands in | 0 |
| 0x074 | u16 | object +0x24 low half (facing angle?) | 0x0398 |
| 0x076 | u16 | object +0x28 low half | 0 |
| 0x078 | 64 × char[12] | inventory item names (11 chars + NUL), restored by name lookup | all empty |
| 0x378 | u16 | object +0x1AE (team number; −1) | 0xFFFF |
| 0x37A | u16 | read back as "current tool" by SGDToPlayerPos but never written by the save routine | 0 |
| 0x37C | i32×16 | state values 0..15 (0 hidden, 1 Life, 2 Magic, 3 Armor, 4 Money, 5 Experience, 6 Power, 7 Hits On, 8 Hits By, 9 Domains, 10 Captures="TF", 11 Kills On="D", 12 Kills By="EK", 13 Time elapsed, 14 Time left, 15 Total score) | 0,100,0,50,100,0,… = log `L=100 MA=0 A=50 MO=100 E=0 TF=0 D=0 EK=0` |
| 0x3BC | u32 | player controller +0x1C | 0 |
| 0x3C0 | u32 | player controller +0x20 (current tool index) | 0 |
| 0x3C4 | … | zero to end of record | |

Restore (GAMEGOTO, FUN_004bd349): if the current slot was saved (bit2), go to its orb and, after
load, SGDToPlayerPos restores position/flags/inventory/states; if only created (GAMENEW), the
orb comes from the GAMEGOTO aux record (`LIPS00`) and only states 0x37C.. are copied into the
player. Copy (FUN_004bc0c1) duplicates a whole record and overwrites index + name.

### Record 0 (header)
LIPSTART.GAM: u32 1 at 0x00, u32 0 at 0x04, `lipstart` at 0x08, rest zero. The loader never
initialises record 0 and GAMESAVE/GAMEGOTO accept "slot 0" when no character is selected
(then PlayerPositionToSGD would write the slot layout above into it). The observed content does
not match that layout (`lipstart` sits where the player flags would be); it matches the first
bytes of the GODDRUN.DAT struct. Writer not located – **open question**. A clean
reimplementation can preserve record 0 verbatim.

### Slots 2–5 in the shipped file
Only the index field is set (2,3,4,5 at record+4); everything else zero = "Not Active".

## GODDRUN.DAT (preferences / run data; goddrun.tad is the 1999 original)

Read by FUN_004b9adb, written by FUN_004b9daa, logical size 0x594 (1428) bytes, file in the
game directory. The read is accepted only if the u32 at 0 equals the version constant at
0x515C8C (= 1); otherwise defaults are used and the "Your Name" dialog is forced.

| off | size | field |
|---|---|---|
| 0x000 | u32 | version = 1 |
| 0x004 | u32 | 0 |
| 0x008 | char[0x98] | last orb name (`lipstart`) [M] |
| 0x0A0 | char[0x100] | startup INI file name (`LIPSTART.INI`; empty in goddrun.tad) [M] |
| 0x1A0 | char[24] | player name (`Your Name` / `YourName`) → config +0x394; empty → name dialog |
| 0x1B8 | char[128] | full name (`Your Full Name Here`) → config +0x3AC; empty → dialog |
| 0x238 | char[24] | config +0x45C (empty) |
| 0x250 | char[0x114] | palette file name (`GoddWinP.pal`) → DAT_007092BC |
| 0x364 | u32 | 1 (constant) |
| 0x368 | u32 | 0 |
| 0x36C | u32 | desktop width at last run (DAT_007093F0): 0xD70=3440 (Wine run) / 800 (tad) |
| 0x370 | u32 | desktop height (DAT_007093EC): 0x5A0=1440 / 600 |
| 0x374 | u32 | render width (DAT_005486A4) 320 |
| 0x378 | u32 | render height (DAT_005486AC) 200 |
| 0x37C | u32 | 0 |
| 0x380 | u32 | 10 (written as `0D 0A 00 00 00` on disk) |
| 0x384 | u32 | 1 |
| 0x388 | u32 | mouse yaw sensitivity (input cfg +0x1C), default 24 |
| 0x38C | u32 | mouse pitch sensitivity (+0x20), default 24 |
| 0x390 | u32 | keyboard L/R sensitivity (+0x18), default 24 (0 → 24 on load) |
| 0x394 | char[24] | config +0x53C |
| 0x3AC | char[128] | config +0x4BC |
| 0x42C | char[24] | config +0x474 |
| 0x444 | char[24] | config +0x48C |
| 0x45C | char[24] | config +0x4A4 |
| 0x474 | char[128] | web host URL (`http://cgi.dnai.com/~brdotwn/cgi-bin/`) → config +0x62C (only if non-empty) |
| 0x4F4 | char[80] | web-chat value name (`gchat-val`) → config +0x79C, lower-cased |
| 0x544 | char[80] | web-chat title (`Beginning New Godd WebCam Chat`) → config +0x7EC |

(File offsets = logical + 1 for everything from 0x381 on, because of the CR LF.)

Anomaly: the Wine-written GODDRUN.DAT is shifted by +1 only for some fields after 0x380 (e.g.
the URL is at file 0x474, the sensitivity bytes are mis-aligned), while goddrun.tad is shifted
consistently. Probably the file was read through a path that did not fold CR LF, so the
in-memory struct was already skewed before being written back – **unresolved**; do not use the
Wine DAT as a reference, use goddrun.tad.
