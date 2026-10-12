# In-game HUD and screen text (lips.exe)

Clean-room notes on everything that is drawn as 2D text on top of the 3D view of the 320x200
game screen: the status HUD (Score / Time / Tool / Treasures / Deaths / Enemies and the
Life…Experience rows), status messages (`XM`), OTEXT text objects, the Talk line, the system
info box, and the five 8x8 glyph blitters. Behaviour is restated from `re/ghidra/decomp.c` /
`decomp_extra.c` (addresses are evidence pointers) and checked pixel by pixel against
`re/shots/original/00_title.png` and `02_newgame.png`.

Confidence: **[H]** read directly from code and confirmed by the screenshot,
**[M]** read from code but not fully traced or not visible in a capture, **[L]** inference.

Related: `script.md` §6.1 (state indices), §6.2 (XM/OTEXT), §8.2 (menu boxes), §11 (BIOS font);
`renderer.md` §1 (views, window keys); `physics.md` §2.6 (debug strings), §5.1 (player ext).

---------------------------------------------------------------------------------------------

## 0. Numbers to remember

| Item | Value | Conf. |
|---|---|---|
| Screen buffer | 320x200, 8 bpp, `DAT_005486a4` = 320, `DAT_005486ac` = 200 (set in the startup routine FUN_004dc382) | H |
| Palette | `original/PALETTE.PAL`, 6-bit RGB. The Wine capture shows `rgb6 * 4` exactly (for example index 120 = (15,32,11) is captured as (60,128,44)) | H |
| Font | 8x8 IBM BIOS ROM font (CP437 glyphs 0..127), 8 px advance, no scaling, no shadow | H |
| Default text width limit | `DAT_0052fe18` = 320/8 − 2 = **38** characters (FUN_004baa5d) | H |
| 3D main view | XRB view 0 is at (0,0), 320x200 in every level → the HUD is drawn **over** the 3D picture. There is no HUD background, panel picture or sub-window | H |
| HUD redraw | every rendered frame, after the 3D view and after the per-view "draw" (event 0x20) hooks | H |
| HUD on/off | `DAT_00510df0`; set to 1 when `[startup] gametype` is CTF/DM1/HNH/TEAMFORT/CONJURE (Lips: HNH → on); F8 (`showplayerstatus`) toggles it | H (flag), M (key) |
| Game type | config +0x3C: 1 CTF, 2 DM1, **3 HNH (Lips)**, 4 TEAMFORT, 5 CONJURE; selects the HUD variant | H |

---------------------------------------------------------------------------------------------

## 1. Where the HUD is drawn (frame order)

Per frame, for each view (FUN_0047242d → FUN_004ab15f for view 0):
1. render the 3D scene into the view;
2. broadcast event 0x20 ("draw") to objects for this view (FUN_00472a54). Through this hook
   the level effect at 0x00456e36 draws the held-tool sprite (bottom centre), the debug text
   picture, overlays (FUN_0045675f), the active menu (FUN_00456aa4, see script.md §8.2) and
   similar;
3. if the view is the main view (view+0x08 == 0, origin ≥ 0) → **overlay text pass
   FUN_004aae49(view)**, in this order (later items overwrite earlier ones):

| # | condition | routine | what |
|---|---|---|---|
| 1 | always | FUN_0043d8d4 | OTEXT text objects (§5) |
| 2 | messages queued and chat log off and show mode ≠ 1/3 | FUN_004c5325 | status message lines, top-left (§4) |
| 3 | chat log on (`DAT_00515674`, alt-L) | FUN_004c54e5 | chat log lines (replaces 2) |
| 4 | `DAT_00510dcc` | FUN_004c7af5 | team table |
| 5 | various debug/panel flags (`DAT_00510dd0..dec`) | FUN_004c6cf3, …, FUN_004c652d (F7 player-states panel), FUN_004c69db (F6 possessions), FUN_004c7179 | other panels, not used in normal Lips play |
| 6 | **`DAT_00510df0`** | **FUN_004c9d43 → per game type** | **status HUD (§2)** |
| 7 | `DAT_00510df4` … `DAT_0050fa40` | FUN_004ca91f, FUN_004ca4bb, … | more panels (debug) |
| 8 | `DAT_00510de0` | FUN_004c5ab9 | debug values |
| 9 | `DAT_0051568a` | FUN_004c5224 | ? |
| 10 | Talk input active (`DAT_00593134`) | FUN_004c611f | Talk line (§6) |
| 11 | `DAT_00515ee8` | FUN_004c58e3 | system info box (§7) |
| 12 | `DAT_00510d78` | FUN_004ca6ea | ? |

The BSP path (FUN_004905d6) calls FUN_004c9d43 and FUN_004aae49(0) in the same way. **[H]**

The "view" given to the text routines is a view-like struct: +0x10 i16 width, +0x12 i16
height, +0x14 pixel pointer (row-major, pitch = width). FUN_004aae49(0) falls back to a struct
that describes the whole 320x200 screen buffer (`DAT_006e38fc`, filled in FUN_004baa5d). For the
normal full-screen view both are the same 320x200 buffer. **[H]**

The HUD is drawn **from scratch every frame** into the freshly rendered frame. There is no
dirty-rectangle logic or "draw only on change". **[H]**

---------------------------------------------------------------------------------------------

## 2. The status HUD (HNH variant FUN_004c88e5, used by Lips)

FUN_004c9d43 picks the variant by game type: 1 → FUN_004c7c4f (CTF), 2 → FUN_004c8353 (DM1),
**3 → FUN_004c88e5 (HNH = Lips)**, 5 → FUN_004c9109 (CONJURE, player vs. opponent columns);
4 draws nothing. Every variant draws nothing unless the player object exists and has a state block
(obj+0x1B4) and a player ext (obj+0x1B8). **[H]**

All strings go through the clipped string blitter `DrawText(view, x, y, colour, bg, str, maxlen)`
(FUN_00475bb4, §3.2) with bg = −1 (transparent, no box). So the 3D picture shows through
around the glyphs.

### 2.1 Exact layout (320x200 coordinates, glyph cell top-left)

Colour names are the PALETTE.PAL colours (×255/63).

| element | x | y | colour (index, rgb) | text / format | maxlen | source | conf |
|---|---|---|---|---|---|---|---|
| "Score" | 280 | 0 | 120 `#3c812c` green | `Score` | 6 | label; **no value in HNH** (team scores only in CTF/DM1: `%3d` at (264,8) and ` %3d` at (288,8), team colour) | H |
| "Time" | 288 | 40 | 120 green | `Time` | 6 | label | H |
| time value | 320 − 8·n (n = visible length; 280 for `mm:ss`) | 48 | 195 `#8d8dff` light blue | `%2.2d:%2.2d` = mm:ss | 8 | seconds since program start (§2.3) | H |
| "Tool" | 288 | 68 | 120 green | `Tool` | 6 | label | H |
| tool value | 304 | 76 | 92 `#9d9d9d` grey | `%d` | 4 | player ext+0x20 = **currently selected tool index** (0..15) | H |
| "Treasures" | 248 | 88 | 120 green | `Treasures` | 14 | label | H |
| "found" | 264 | 96 | 120 green | `  found` (2 leading blanks → visible text starts at x 280) | 14 | label | H |
| treasures value | 296 | 104 | 92 grey | `%3.3d` | 4 | player state[10] (statebuf+0x1C0) | H |
| "Deaths" | 272 | 118 | 120 green | `Deaths ` (trailing blank) | 12 | label | H |
| deaths value | 296 | 126 | 92 grey | `%3.3d` | 4 | state[11] (+0x1C4) | H |
| "Enemies" | 264 | 138 | 120 green | `Enemies` | 12 | label | H |
| "killed" | 264 | 146 | 120 green | ` killed` (1 leading blank → visible text starts at x 272) | 12 | label | H |
| kills value | 296 | 154 | 92 grey | `%3.3d` | 4 | state[12] (+0x1C8) | H |
| heart icon | 1 | 181 | 176 `#ff0000` red | char 0x03 (CP437 heart) | 2 | icon | M (blank in capture, see §2.4) |
| "Life:" | 8 | 181 | 232 `#a53c00` orange | `Life:` | 6 | label | H |
| life value | 64 | 181 | 92 grey | `%3.3d` | 8 | state[1], rolling display (§2.2) | H |
| spade icon | 120 | 181 | 251 `#ff00ff` magenta | char 0x06 (CP437 spade) | 2 | icon | M |
| "Magic:" | 128 | 181 | 232 orange | `Magic:` | 6 | label | H |
| magic value | 176 | 181 | 92 grey | `%3.3d` | 8 | state[2], rolling | H |
| diamond icon | 240 | 181 | 199 `#1818ff` blue | char 0x04 (CP437 diamond) | 2 | icon | M |
| "Armor:" | 248 | 181 | 232 orange | `Armor:` | 6 | label | H |
| armor value | 296 | 181 | 92 grey | `%3.3d` | 8 | state[3], rolling | H |
| "$" | 1 | 190 | 123 `#245018` dark green | `$` | 2 | icon | H |
| "Money:" | 8 | 190 | 232 orange | `Money:` | 8 | label | H |
| money value | 64 | 190 | 92 grey | `%6.6d` | 11 | state[4], rolling | H |
| "*" | 176 | 190 | 230 `#ffff20` yellow | `*` | 2 | icon | H |
| "Experience:" | 184 | 190 | 232 orange | `Experience:` | 12 | label | H |
| experience value | 272 | 190 | 92 grey | `%6.6d` | 11 | state[5], rolling | H |

Notes on the coordinates:
* The code positions the two bottom rows relative to the screen height: row 1 at `H − 19`
  (= 181) and row 2 at `H − 8` (= 192). The string blitter forces any `y` with `y + 10 ≥ H` to
  `H − 10`, so **row 2 is really drawn at y = 190**. The capture confirms 181 and 190. **[H]**
* `%3.3d` / `%6.6d` are "at least 3 / 6 digits, zero padded" (`100`, `050`, `000100`). Larger
  numbers get more digits, up to the maxlen limit. Negative values print as `-01` and so on.
* Labels and values are separate draw calls. The value x positions are fixed. They do not depend
  on the label length.
* Capture check (png → screen: `x = (px−80)/1.478`, `y = (py−60)/1.635`; the game image fills
  png x 80..552 and y 60..386): "S" of Score at (494,60) → (280,0); "T" of Time at (506,126) →
  (288,40); time digits at (494,139) → (280,48); "L" at (92,356) → (8,181); "M" of Money at
  (92,371) → (8,190); "$" at (82,371) → (1,190); Life value at (176,356) → (65,181);
  Experience value at (483,371) → (272,190). All colours in the capture map exactly to palette
  indices 120, 92, 195, 232, 123 and 230. **[H]**

ASCII sketch (one char = 8 px, `.` = 3D view showing through):

```
x→ 0       64      128     192     256  296 320
y0   ........................................Score
y40  ............................................Time
y48  ...........................................00:05
y68  ............................................Tool
y76  ..............................................0
y88  ...............................Treasures
y96  ...................................found
y104 .....................................000
y118 ..................................Deaths
y126 .....................................000
y138 .................................Enemies
y146 ..................................killed
y154 .....................................000
y181 ♥Life:   100     ♠Magic:000      ♦Armor:050
y190 $Money:  000100              *Experience:000000
```

### 2.2 Rolling counters (Life, Magic, Armor, Money, Experience) **[H]**

The five bottom values are not printed directly. A "displayed value" array `shown[16]`
(`DAT_0053d070`, indexed by state number) moves **one unit per HUD draw** toward the real value.
Per frame, for state i:

```
old = shown[i]
if old < state[i]:   shown[i] += 1; print old
elif old > state[i]: shown[i] -= 1; print old
else:                print state[i]
```

So the number counts up or down by 1 per rendered frame, and the frame prints the value from
before that step. `shown[]` is zeroed only at engine initialisation (FUN_004baa5d), not per level.
At the first game start the bars count up from 0 (Life 0→100, Armor 0→50, Money 0→100). Big
money/experience jumps take one frame per unit. Treasures/Deaths/Kills, Tool and Time are printed
directly.
All HUD variants share the same `shown[]` array (the CONJURE variant has a second one for the
opponent, `DAT_0053cff0`).

### 2.3 Time **[H]**

HNH uses FUN_00455017: `t` = the engine's whole-second clock `DAT_006cc46c` (FUN_0049fbe4), shown as
`minutes = (t mod 3600) / 60`, `seconds = t mod 60`, formatted `%2.2d:%2.2d`. So it wraps every
hour and has no hours field. The clock is reset only once, at program start (FUN_0049f8bb, called
from FUN_004dc382). The HUD time is therefore **wall time since the game was launched**, not
state 13 and not a per-level timer (the capture shows 00:05 on the title menu and 00:09 a little
later in the first level). The seconds counter is advanced from GetTickCount in 1000 ms steps,
and each step drops the leftover milliseconds, so it runs slightly slow (FUN_0049fa1c). A
reimplementation can use `floor(elapsed_ms/1000)`.
The CTF/DM variants use FUN_004550a9 instead: a countdown `(gameLimit − t) mod 3600`.

The time string is right-aligned. FUN_004c21fc counts the visible characters of the string
(skipping 0xFE colour pairs), and x = 320 − 8·count.

### 2.4 Icons and the Wine capture **[M]**

The icons are control characters 0x03, 0x06 and 0x04. In the real IBM 8x8 BIOS font these are the
card suits ♥, ♠ and ♦ (0x1E/0x1F ▲▼ mark flags in the CTF HUD). The capture was made with the
injected substitute font `re/run/biosfont.bin`. That font has empty glyphs for 0x00–0x1F, so the
icons are missing from the screenshot. A reimplementation must embed a full CP437 8x8 font,
including 0x01–0x1F.

### 2.5 Other game-type variants (not used by Lips) **[M]**
* DM1 (FUN_004c8353): Score, Time (countdown), Tool. The Life/Magic/Armor row is at `H−8` (clamped
  to 190), with values at x 48/176/296 and the spade icon in colour 123. There is no second row.
* CTF (FUN_004c7c4f): as DM1, plus team scores and flag carrier names (▲/▼ at x 304, y 156/168).
* CONJURE (FUN_004c9109): player and opponent Life/Magic/… columns at `H−46`, `H−38`, ….

---------------------------------------------------------------------------------------------

## 3. Text rendering primitives

All glyph data come from the BIOS font address `char*8 + 0xFFA6E` (script.md §11). A glyph is 8
bytes, one per row, top to bottom. Bit 0x80 is the leftmost pixel. No scaling and no shadow are
used anywhere.

### 3.1 The five glyph blitters

| function | args | target | clipping | pixels |
|---|---|---|---|---|
| **FUN_00475ab1** | (view, x, y, colour, ch) | the view buffer `view+0x14`, pitch = view width (+0x10) | drawn only if `x+8 ≤ W` and `y+8 ≤ H`, else the whole glyph is skipped; no test for negative x/y (the caller clamps) | **transparent**: only set bits are written, in `colour` |
| **FUN_0046ce0f** | (x, y, colour, ch) | the global screen buffer `DAT_005107d8`, pitch 320 | drawn only if `x ≥ 0`, `y ≥ 0`, `x+8 ≤ 320` and `y+8 ≤ 200` | transparent |
| **FUN_00475d7f** | (x, y, colour, ch) | screen buffer `DAT_005107d8` | `x+8 ≤ 320`, `y+8 ≤ 200` | **opaque**: set bits get `colour`, clear bits get **0** (black) → solid 8x8 cell |
| **FUN_00475eda** | (buf, x, y, colour, ch) | any 320-pitch buffer (used with the raw "transit" screen `DAT_005107ec`) | `x+8 ≤ 320`, `y+8 ≤ 200` | opaque (0 behind) |
| **FUN_0047550a** | (picRef, x, y, colour, ch) | the pixels of a cached **picture** (texture), which are stored **column-major**: one step down = +1 byte, one step right = +column length | `x+8 ≤ picW`, `y+8 ≤ picH` | transparent. It also stamps the picture's cache entry with the frame counter and reloads it if it was purged |

The string wrappers are FUN_0047563e (FUN_0047550a per char), FUN_00475e7e (FUN_00475d7f), FUN_00475fd9 (FUN_00475eda),
FUN_0046d0eb (FUN_0046ce0f with colour codes and box) and **FUN_00475bb4** (FUN_00475ab1 with colour codes and box). The
plain wrappers advance 8 px per char and have no colour codes and no clipping beyond the per-glyph test.

Users: FUN_00475bb4 → all HUD/message/OTEXT/Talk/hotspot text. FUN_0046d0eb → menu title and
subtitle (script.md §8.2). FUN_00475e7e → loading progress `"%s %d"` at (8, H−12) and a debug
line at (4,4), both colour 168 (white). FUN_00475fd9 → the built-in key-help screen ("ESC - Quit",
"F11 - Menu", …, colour 4 = white) in FUN_0046f675. FUN_0047550a/FUN_0047563e → the debug text
list drawn into the picture `DAT_00543ce4`. **[H]**

### 3.2 The clipped string blitter `DrawText` (FUN_00475bb4) **[H]**

`DrawText(view, x, y, colour, bg, str, maxlen) → final colour`

```
n = min(maxlen > 0 ? maxlen : 38, strlen(str))      // raw bytes, colour codes included
boxW = 8*n + 6;  if x + boxW > W: boxW = W - x
if y + 10 >= H: y = H - 10
if y < 2:       y = 0
if x < 0:       x = 0
if bg >= 0 and n > 0: FillRect(view, x, y-2, boxW, 12, bg)   // FUN_004759ad, skipped if w or h <= 1
col = 0
i = 0
while i < n and str[i] != 0:
    if str[i] == 0xFE:
        colour = str[i+1]; i += 2
        if str[i] == 0: return colour
    glyph(view, x + 8*col, y, colour, str[i])         // FUN_00475ab1 (transparent)
    col += 1; i += 1
return colour
```

* The 0xFE byte and its colour byte take no screen space. The **next byte after a colour pair is
  always drawn as a glyph**, so two pairs in a row do not work. The `maxlen` limit and the box
  width count raw bytes, so a colour-coded string gets a box that is 2 chars wider per code.
* The returned colour lets callers carry a colour code across wrapped lines.
* The background box starts 2 px above the glyph row and is 12 px tall (y−2 … y+9).
  FillRect clips rows at the screen bottom and stops at the end of the buffer.
* `FUN_0046d0eb` (screen version, used by the menu) is the same, except that its y clamp is
  `y + 13 ≥ 200 → y = 187`.
* `FUN_00475d1e(view, x, row, …)` = `DrawText` at `y = row*8`, then a global line counter
  `DAT_00515ee4` += 1. The message and chat routines use it for line stacking.

---------------------------------------------------------------------------------------------

## 4. Status messages (`XM`, announcements)

### 4.1 Building and posting **[H]**
* `XM(col,"text")` (FUN_0040f426): colour = col or 92 if 0/omitted. FUN_00453951 builds
  `0xFE, col, text…` in the buffer `DAT_00707434` (it can chain up to five colour+text parts, but
  XM uses one). FUN_004b15c2 posts it.
* Posting (FUN_004b12cb) takes one of **20 message slots** (0x114-byte records at `0x006F72C0`):
  text copied (max 210 bytes) to slot+0x3C, default colour slot+0x110 = 60 (`#de9161`, used
  only when the text has no 0xFE code), creation time slot+0x34 = seconds clock, expiry
  slot+0x38 = `created + 3 + visibleLen/9`. The slot is appended to an ordered list (`DAT_006F7270`,
  max 20, the oldest is pushed out when full) and also copied to the chat log (colour 0xE3).
* XM then **overrides the expiry to `created + 3` seconds**. Other announcements (FUN_004b149e:
  OTEXT in popup mode, GAME* texts, …) use `created + max(n, 3)` seconds, usually n = 5.
* Each frame FUN_004b1b3c removes every slot whose expiry < now (list compacted, order kept).
  The test uses whole seconds, so a "3 s" message stays visible for 3–4 s.

### 4.2 Display **[H]**
The engine has a "show mode" (config +0x48): 2 = FULLSCREEN (`[startup] lastshowmode=FULLSCREEN`,
which Lips ships), 1/3 = windowed or chat styles. In windowed styles every message is a small
GDI pop-up window (class "Godd Chat"). **In FULLSCREEN mode the messages are drawn into the frame**
by FUN_004c5325 whenever the chat log is off:

* Up to the **first 5 entries of the list (the oldest still alive)** are drawn, top-down from
  y = 0. The line counter starts at row 0 every frame, and each output line is 8 px lower.
  No background box. Colour = the 0xFE code in the text (XM), otherwise slot colour (60), and 168
  if that is 0.
* Wrapping: a line holds at most 38 bytes. If the text is longer, the break is moved back to just
  after the last blank within the first 38 bytes. If that leaves fewer than 2 chars, it is a hard
  break at 38. Continuation lines are **indented 4 chars (x = 32)** and hold 34 bytes. The colour
  carries over to the continuation lines.
* So messages appear at the **top-left corner**: message k on rows 8·k… (more if earlier ones
  wrapped). They do not scroll. When the oldest expires, the rest move up one entry. A 6th
  message waits (it is alive but not drawn) until a slot in the first five frees up.

Script consequence: commas in XM text become blanks (script.md §3). The final string, after the
2-byte colour code, is what is measured against 38.

### 4.3 Chat log (alt-L) **[M]**
When `DAT_00515674` is set, FUN_004c54e5 draws 8 lines from the chat list box instead of
messages. The text up to and including a `:` (the speaker) is drawn in the line's colour (default
232). The rest is drawn in 84 (`#d2d2d2`), with the same 38/34 wrapping and x offset 4.

---------------------------------------------------------------------------------------------

## 5. OTEXT text objects

### 5.1 Data **[H]**
OTEXT objects (effect OTEXT = FUN_0043e612, 302 in the shipped XRBs) use their aux record:

| aux | meaning | shipped value |
|---|---|---|
| +0x88 | x | 0 |
| +0x8C | y | 158 (one object 160, the LANTERN pickup 0) |
| +0x90 | not used by the drawing code (other effects use it as a parameter) | 32 (one 4) |
| +0x94 | display time in seconds (≤ 0 → 5) | 5 |
| +0x98 | show count: `mod 1000` = how many times it may be shown (0 = unlimited) | 0 or 1 |
| +0x4A,+0x56,+0x62,+0x6E,+0x7A | five 12-byte text slots, concatenated; **`_` → space** (FUN_0043d48d) | e.g. `YOU_FOUN`,`D_THE_LA`,`MP_OF_NI`,`CENESS!` → "YOU FOUND THE LAMP OF NICENESS!" |
| +0x9C | sound played when shown (aux sound slot 0) | POP01, KNOCK01, … |

Object +0x54 selects where the text comes from. 0 = the aux slots (all shipped objects). 4 = a
`[namedmessages]` key in slot +0x4A. 5 = a 128-byte string buffer at obj+0x1A4 (the `_DEF_OT_`
object of the `OTEXT("…")` command; copied raw, so `_` is **not** converted there). 1/2 = a
multi-line text section named by slot +0x7A (FUN_0043dcb6), one line every 13 px. 3 = text taken
from another object.

### 5.2 Creation-time y adjustment **[H]**
On creation (event 1), if the HUD is on and y > 115, the y position is moved up to clear the
HUD's bottom rows: by **32 px for HNH** and by 16 px for CTF/DM1. So in Lips the shipped
y = 158 becomes **126**, and `_DEF_OT_` (y = 170, script.md §6.2) becomes **138**.

### 5.3 Showing and drawing **[H]**
* Trigger (tag event 0x40, also 0x80/0x200/0x80000 from bumps/use when the object is not
  disabled): if it is not already showing and the show count allows it (counter obj+0x11C,
  initialised from +0x98 mod 1000 and decremented per show; nothing happens once it reaches 0),
  set expiry obj+0x118 = now + aux+0x94 seconds, mark visible (obj+6 bit 2) and play the sound.
  Any other visible text object whose y is within [y_this−8, y_this] is hidden right away
  (FUN_0043e551) → a new text replaces an old one on the same line.
* Event 0x10 (per tick) and the draw pass hide it once now > expiry. Text stays visible for
  **5 s** by default (whole-second clock → 5–6 s).
* Drawing (FUN_0043d8d4, step 1 of the overlay pass): for each visible text object,
  `DrawText(view, x + offX, y + offY, colour, bg, text, strlen)` with the global text style:

| global | default | meaning |
|---|---|---|
| `DAT_005100a8` | **176** (`#ff0000` red) | text colour |
| `DAT_005100ac` | **0** (black) | background box colour (≥ 0 → a black `8n+6 × 12` box is drawn behind the text) |
| `DAT_005100b0/b4` | 0, 0 | x/y offset |
| `DAT_005100b8` | 0 | mode: 0 = draw in frame, 1 = post as an announcement for aux+0x94 s (§4) instead |

  The style can be changed by an OTEXTSET object (FUN_0043e3fc copies its aux +0x88..+0x98 into
  these globals on trigger). No shipped level has one, so Lips text is **red on a black box at
  (0,126)**.
* There is no word wrap for single-line OTEXT. The box is clipped at the right screen edge, and
  glyphs past x 312 are dropped (40 chars fit). Multi-line (kind 1/2) text uses the line length
  stored in the text-section object.

---------------------------------------------------------------------------------------------

## 6. Talk / command input line **[H layout, M keys]**

Talk mode starts when `T` (Talk), `R` (TeamTalk), `I` (Individual) or the console is used
(keyboard.txt; FUN_004e8325/FUN_004e840c set `DAT_00593134` = 1 and the prompt kind
`DAT_005931e0`). The same line is also used for engine text prompts such as "Enter Character Name:"
(prompt kind 6 = custom prompt text) **[M]**. FUN_004c611f draws:

* text = prompt (`"Talk: "`, `"Command: "`, `"TeamTalk: "`, `"Individual: "` or the custom prompt) +
  typed text + `"_"` (cursor). If it is longer than 37 visible chars, only the **last 37** are
  shown (it scrolls left).
* vertical offset d = 21 for HNH (6 for CTF/DM1, 0 otherwise). With H = 200, W = 320 in Lips:
  * black filled box: x 1, y `H−15−d` = 164, w `W−5` = 315, h 13;
  * frame (4 lines, FUN_0046cc65): x 1, y `H−16−d` = 163, w `W−4` = 316, h 15, colour 178
    (`#e20000` dark red);
  * text: x 4, y `H−12−d` = 167, colour 92 (grey), bg 0 (adds a black box behind the text).

So in Lips the Talk line sits just above the Life/Magic/Armor row (y 163..177).

## 7. System info box (FUN_004c58e3) **[H]**

Used for console results ("Command … Done"), "No Available Game Slots / Delete or Use existing
game", "Screen Snap to …", "Who is …", password notices and so on (FUN_004c57d4(line1, line2,
line3, seconds, mode)). n = number of lines (1–3), top = `H − (15n + 32)`:
frame rectangle colour 178 at (2, top−2), size (W−6) × 15n; lines at x 4, y top+2, top+14, top+26,
colour 4 (white), transparent, each showing at most the last 37 chars. It disappears `seconds`
after it was set (2–5 s).

---------------------------------------------------------------------------------------------

## 8. Toggles and related keys

| action (key per keyboard.txt) | handler | effect | conf |
|---|---|---|---|
| `showplayerstatus` (**F8** "toggles the status display") | FUN_00477d06 | toggles `DAT_00510df0` (the §2 HUD). When it is switched on, the possessions panel (`DAT_00510dec`) and the `DAT_00510df4/de4/e04` panels are switched off | H (handler), M (key) |
| `showplayerstates` (F7, "detail of player states") | FUN_00477c8c | toggles `DAT_00510de8` → FUN_004c652d, a list of state names and values (`"%s %d"`, Life…Time left). Switches the other panels off | H / M |
| `showpossessions` (F6) | FUN_00477bc6 | toggles `DAT_00510dec` → FUN_004c69db | H / M |
| alt-L | | chat log (§4.3) on or off; messages are hidden while the log is on | M |
| window keys (`inc/decviewwidth/height`, `setfullviewsize`) | renderer.md §1 | resize/recentre the 3D view. The HUD still uses its fixed coordinates and is drawn into the main view's struct | H (keys), L (effect on HUD) |

The action table at 0x0051310F holds (handler, name) pairs. The default key → action bindings
were not located in code. The F-key assignments above come from `original/keyboard.txt`.

Menus: box, title, rules and hotspot text are described in script.md §8.2 (FUN_00441c6f,
FUN_0046d0eb, FUN_00475bb4). Menus are drawn in the event-0x20 draw hook, before the HUD pass,
so HUD text would overwrite a menu box where they overlap **[M]**. In the shipped menu (title
capture) they do not overlap.

XRB **Overlays** (FUN_0045675f): all shipped overlay records have texture −1 and a clear active
bit (record +4 bit 0), so nothing is drawn. No picture sits behind the HUD. **[H]**

---------------------------------------------------------------------------------------------

## 9. Reimplementation summary

1. Render the 3D view at full 320x200 (view 0 = (0,0,320,200)). Then run the per-frame text
   pass in this order: OTEXT objects → status messages → HUD → Talk line → info box. Everything
   is drawn into the 8-bit frame with the CP437 8x8 font (full 0x00–0x7F, including the suit
   symbols), using transparent glyphs.
2. Implement `DrawText(x, y, colour, bg, str, maxlen)` exactly as in §3.2: the y clamps (`y+10 ≥ 200 →
   190`, `y < 2 → 0`), the optional `8n+6 × 12` box at y−2, the 0xFE colour codes, and the returned
   colour.
3. HUD (when `hud_on`, default on, F8 toggles): draw the table in §2.1 every frame. Bottom
   values use the ±1-per-frame rolling counters of §2.2 (state 1..5). The right column shows
   states 10/11/12 (`%3.3d`), the selected tool index (`%d`), and mm:ss of seconds since program
   start (right-aligned to x = 320, colour 195). "Score" is a bare label in Lips.
4. XM/announcements: queue of max 20 entries `{text with FE code, expiry}`. XM expiry = now+3 s,
   other announcements now+max(n,3) s. Draw the 5 oldest at the top-left, 8 px per line, wrapping at 38
   chars on a blank with 4-char indented continuations, no box. Remove an entry when its expiry
   (whole seconds) is passed.
5. OTEXT: text = concatenated aux slots with `_`→space (raw for the `OTEXT("…")` buffer). y > 115
   → y −= 32 at creation (HNH with the HUD on), giving y 126 / 138. Show for aux+0x94 s (5),
   respecting the show count. Draw red (176) on a black box at (x, y). A new text hides others
   within 8 px above it.
6. Talk line: black box (1,164,315,13), red-brown frame 178 (1,163,316,15), prompt+text+`_` at
   (4,167) in grey 92 on black, showing the last 37 chars.

## 10. Open questions

1. Default key bindings: the F8/F7/F6 and T/R/I assignments come from keyboard.txt. The
   binding table in the exe was not found.
2. When the 3D view is shrunk with the window keys, is the HUD still drawn at screen coordinates?
   (FUN_004aae49 gets the main view struct; whether that struct's buffer is the screen or a
   private view buffer was not traced.)
3. How the 320x200 frame is scaled to the 472x328 area seen in the Wine capture (the
   DirectDraw/GDI blit path). renderer.md §11 Q1 remains open, but it does not affect the
   320x200 layout.
4. Exact draw order between the menu (event-0x20 hook) and the HUD was inferred from call order,
   not verified in a capture with an overlap.
5. The OTEXT aux +0x90 value (32/4) is unused by the drawing code. It may be read by the
   multi-line text path (kinds 1/2), which is not used by Lips.
6. Panels FUN_004c652d (F7) and FUN_004c69db (F6) were not laid out in detail.
