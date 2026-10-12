# G.O.D.D. INI script language (lips.exe)

Reference for the per-level `LIPSnn.INI`, `LIPSTART.INI` and `lips.ini` files as interpreted by
lips.exe (Watcom build, Dec 1999). Everything here is described from behaviour observed in the
decompilation (`re/ghidra/decomp.c`), extra decompilation of the command handlers that Ghidra had
not turned into functions (`re/scripts/DecompAt.java`-style run, scratch only), the shipped data, and
`GODDRUN.LOG`. Inventory of what the shipped files use: `python3 -I re/scripts/ini_commands.py original`.

Confidence tags: **[H]** read directly from code paths, **[M]** inferred from code + data,
**[L]** guess from names/usage only.

---------------------------------------------------------------------------------------------------

## 1. Files and sections

| file | role |
|---|---|
| startup INI (`lips.ini` / `LIPSTART.INI`) | `[startup]` keys are read from it with `GetPrivateProfileString`; its `[nostrap9999]`/`[commands9999]` run once at game start. Buffer `DAT_007091a4`. GODDRUN.DAT stores the name `LIPSTART.INI`. **[M]** |
| orb INI (`<orbname>.INI`, buffer `DAT_00708368`) | `[moveractioncodes]`, `[namedmessages]`, `[nostrap9999]`, `[commands9999]` of the level being loaded; also `[startup] prologtype/commandline` and text sections pulled by text objects (FUN_0043dcb6 mode 2). **[H]** |

Sections used by the shipped files (all 40 copies incl. LIPSINI.ZIP; the 18 level INIs are
byte-identical except `orbname`, `loadbspfilename` and `OSETJUMP` in LIPS03):

| section | reader | notes |
|---|---|---|
| `[startup]` | Win32 `GetPrivateProfileString/Int` (FUN_0046fc5b, FUN_00470xxx @ decomp 32149-32570) | plain `key=value`, no pre-processing |
| `[nostrap9999]` | FUN_0046ed65 → FUN_00449f3b → FUN_0040b545 per line | only at a *fresh* start (see §5) |
| `[commands9999]` | same | at start **and** every time the orb is (re)loaded |
| `[moveractioncodes]` | FUN_0044a718 → FUN_0044a4c6 → FUN_0044a3f8 | mover ("bot") scripts, separate mini-language (§9) |
| `[namedmessages]` | FUN_0046ea7e → FUN_0046e755 | string table (menus, OTEXT etc.), §8 |

Section text is fetched with `GetPrivateProfileSection` into a growing buffer (1 KB steps,
FUN_0046e3bc). Consequence: the Win32 profile parser trims leading/trailing blanks of each
value – that is why menu strings are written `m1title="   text` (a leading `"` protects the
indentation; the engine strips one leading and one trailing `"`). **[M]**

## 2. Pre-processing of command sections (FUN_00449f3b) **[H]**

Applied to `[nostrap9999]`, `[commands9999]` and `[moveractioncodes]` before anything is parsed:

1. `;` starts a comment that is blanked up to the end of that line.
2. `\` is blanked, as is everything after it on the line, and the line break is turned into a
   space → explicit line continuation.
3. While inside `{ … }` (nesting tracked, max 16) line breaks become spaces → a brace block may
   span lines without `\`.
4. Compaction pass: leading blanks dropped, runs of blanks collapsed to one, and **every
   lowercase letter `a`–`z` outside double quotes is deleted** (quoted text is copied verbatim).

Rule 4 is load-bearing: `Experience=0` becomes `E=0`, `SUMOS(-5,Armor,Life)` becomes
`SUMOS(-5,A,L)`, `SetHitstokill(1)` becomes `SH(1)`, `OSOUND (GROWL15)` keeps working. Authors
evidently wrote mixed-case "documentation" names knowing only the capitals survive. Confirmed in the
disassembly at 0x44a149 (`cmp al,'a' / cmp al,'z'` → skip store).

`[namedmessages]` and `[startup]` are **not** pre-processed (lowercase survives, `;` lines are
skipped by the table builder).

## 3. Command line execution (FUN_0040b545 / FUN_0040b422) **[H]**

Each logical line (and each stored ONTAG body, §4) is executed as a *sequence* of commands:

* The line is copied to a 199-byte buffer (longer lines are truncated at 198 chars).
* Loop: read a command word = characters up to one of ` ,;?({)}="\r\n`, upper-cased, max 22
  chars (FUN_0040984c). `[` is *not* a delimiter, so `ONTAG[401]` / `OPTOOL[3]` are one word.
* Look the word up (with any `[...]` suffix temporarily cut) in the command table at
  `0x0050FA48` (pairs name→handler, list in §6). **Unknown word → the rest of the line is
  silently abandoned.**
* Argument text: skip blanks and one `=`; if the whole remaining line ends in `)` or `}` that
  last char is removed (once); if the argument starts with `(` or `{` the matching bracket is
  found and the handler only sees that group; otherwise the handler gets the rest of the line.
* Handler is called with (command word incl. `[n]`, argument text). Most handlers first run
  FUN_0040b706 on the argument: skip blanks, one `=`, one `(`, blanks, then **every `,` after
  the first character becomes a space – including commas inside quoted strings** (so
  `XM("Careful, your LIFE…")` displays `Careful  your LIFE…`). **[H]**
* After the handler, parsing resumes after the bracket group (one separator skipped).
* Result codes are only logged when the console invoked the command ("Command … Done/FAILED").

Typo consequence: the shipped `ONTAG[261..270]={ OS0UND(GORE00n) }` (zero instead of O) never
plays anything – `OS0UND` is not in the table so the body is dropped. **[H]**

## 4. Tags

### 4.1 Activation (ActivateTag = FUN_00472b7c) **[H]**

`ActivateTag(t)`:
1. Re-entrancy depth counter; more than 12 nested activations → logged and ignored.
2. If `1 ≤ t < 999` and an ONTAG body is stored for `t` (table `DAT_0056c1f4[t]`), run it
   (FUN_00472b0d: strip a leading `{`, normalise with FUN_0040b706, execute as §3).
3. Then, if `t ≥ 999`, or `t == 9999`, or the "tag referenced" byte `DAT_0056bd8c[t]` is set
   (set when a mover script's `T(..)` condition or similar mentions the tag), scan the world:
   * every object whose tag field (object +0x44, 16.16 – integer part) equals `t` → call its
     effect handler (object +0x140) with event **0x40**;
   * every wall with tag (+0xA4) → effect at +0xD0, event 0x40;
   * every region with tag (+0x84) → effect at +0xB0, event 0x40.
4. Tags 8000–8999 additionally go to FUN_0048ee77 (network / player-message range) **[L]**.

So a tag is a broadcast number: ONTAG scripts and XRB objects/walls/regions listening on the
same number all fire. An effect only reacts if its event mask (column "flags" in
`effects.md`) contains 0x40 and the object is not disabled.

### 4.2 Who fires tags

| source | how |
|---|---|
| `TAG=n` command | immediate ActivateTag(n) (LIPSTART: `TAG=600` → main menu object MENU1 has tag 600) |
| `XT(n)` / `XT(a-b)` | ActivateTag(n) or a uniform random tag in a..b (b≤a gives tag 1) – FUN_004445b5 |
| OS thresholds | SETMAX/MIN/LOW/HIGH-OSTAGS, fired by INCOS/SUMOS/XCHANGE changes of the *player* (§7) |
| XCHANGE / OTOOLCON | success / failure tags |
| mover scripts | `XT(..)` inside ON-blocks (bumps, hits, kills) |
| XRB objects | effect aux record success/fail tags (+0x94/+0x98), e.g. GAMENEW, GAMESAVE, hotspots |
| engine | 9999 at every (re)start before `[commands9999]` (FUN_004bb0b3, FUN_004bb442) |

### 4.3 Numbering conventions in the shipped game **[M]**

| range | meaning (from INI comments + data) |
|---|---|
| 250–258 | bot death sounds, chosen by `XT(250-258)` from tag 761 |
| 260–271 | player gore sounds (`XT(260-271)` from 661; bodies use the `OS0UND` typo → silent) |
| 401–409 | pickups dropped by dead bots 1–9 (`DEADBOTn ON BP → XT(40n)`): INCOS + sound |
| 411–419 | player hit by bot n (`P(1,41n)` in bot code): SUMOS damage absorbed by armour, XM taunt |
| 501–518 | exchange bots: odd = refused (not enough), even = done |
| 600–629 | LIPSTART menu system: 600 main menu, 601 GAMENEW, 602 GAMECUR (select slot), 610 menu 2, 612 GAMECOPY, 613 GAMEDEL, 619 GAMEGOTO (start), 620 quit menu, 621 GAMESAVE, 623 GAMEGOTO/return, 629 OQUIT |
| 661 | player dies: SETMINOSTAGS LIFE, and bots' `ON BP {… XT(661)}` instant kill |
| 662–665 | MAGIC/ARMOR/MONEY/Experience reached minimum (no ONTAG defined) |
| 671–674, 680 | low-value warnings (LIFE<=25, MAGIC<=30, ARMOR<=10, MONEY<=0); 674 = not enough magic for tool 8 (OTOOLCON) |
| 675, 685 | Experience ≥300 / reached max (no ONTAG defined) |
| 761 | a bot was killed/hit → random death sound |
| 9999 | "startup" broadcast |

## 5. Start-up / evaluation order **[H unless noted]**

Fresh start of the game (FUN_004bb0b3):
1. ActivateTag(9999).
2. startup INI: `[nostrap9999]`, then `[commands9999]`.
3. orb INI: `[nostrap9999]`, then `[commands9999]`.

Loading a new orb in-game ("OneCycle Strapping LoadNewOrb", FUN_004bb442) and the equivalent
path at ~decomp 66280:
1. load XRB, `[moveractioncodes]` (FUN_0044a718), `[namedmessages]` (FUN_0046ea7e);
2. ActivateTag(9999); orb INI `[commands9999]` only — **`[nostrap9999]` is skipped**, so the
   initial `SETOS=(LIFE=100,…)` of a level runs only when the level is the first one started;
3. if neither a save slot restore nor a "go to game" restore is pending, the player's 16 state
   values are restored from the carry-over copy (`DAT_0053d030`, FUN_0044b106) taken before the
   level change (FUN_0044b16a) → states persist across levels;
4. pending save-slot restore (SGDToPlayerPos FUN_004bcfc6) or state copy (FUN_004bcdc7);
5. ActivateTag(9999) again.

Within a section, lines run top to bottom; ONTAG lines only *store* bodies. SETMAX/MIN… tables
must be set before INCOS can change anything (max defaults to 0, see §7).

`[startup]` keys (decomp 32149-32570; read from the startup INI unless noted):

| key | effect |
|---|---|
| `orbname` | first orb to load (extension stripped); copied to game-config +0x27C |
| `goddflags` | engine flag letters, default `MWKDH`; Lips uses `SFWD` **[L: letters not decoded]** |
| `debugflags`, `renderflags`, `speakerflags` | more flag strings (unused by Lips) |
| `commandline` | extra command-line switches (`-l`) |
| `gametype` | `CTF`=1, `DM1`=2, `HNH`=3, `TEAMFORT`=4, `CONJURE`=5 (config +0x3C); Lips = `HNH` |
| `savegame` | `Y…` enables the save-game system (LIPSTART.GAM) |
| `startgameorbname` | orb to return to for game select (`lipstart`), config +0x314; enables GAMEGOTO "return" |
| `loadbspfilename` | BSP to load with the orb (`lips00c.bsp`; `.BSP` appended if no extension) |
| `titlescreen0..2`, `screentime0..2` (2/10/10 s), `showmethod0..2`, `titlesound0..2` | splash bitmaps |
| `endscreen`, `endtime`, `endshowmethod`, `endsound` | exit bitmap |
| `defaultorbname`, `lastorbname`, `lastshowmode`(=FULLSCREEN), `doorbellsound`, `graphicsmode`(=640x480), `playerscolorname`, `picslibs`, `kbLRsensitivity`, `mouseyaw`, `mousepitch`, `*lightmin`, `walldistinside/outside`, `maxflyheight`, `optimalframerate`, `freefallhighdist`, `maxsounddist`, `midsounddist`, `activatedistance`, `playerspeedmax{x,y,z,t}`, mail/web keys | engine options, not used by Lips |

## 6. Command reference

Arguments after pre-processing; "player" = local player object (`DAT_00510778`), "opponent"
= object set by `OPPONENT` (`DAT_00510780`). Names in *italics* are not used by the shipped INIs.

### 6.1 Object-state ("OS") commands

The player (and any object with a state block, object+0x1B4) carries 16 ints at
statebuf+0x198, index 0..15. Display names (table @0x00510230): 0 hidden, 1 **Life**,
2 **Magic**, 3 **Armor**, 4 **Money**, 5 **Experience**, 6 Power, 7 Hits On, 8 Hits By,
9 Domains, 10 Captures, 11 Kills On, 12 Kills By, 13 Time elapsed, 14 Time left, 15 Total
score. Lips re-uses 10/11/12 as "TF / D / EK" (treasures found, deaths, enemies killed – the
labels printed in the save log; 10 += 1 on loot pickup, 11 += 1 in ONTAG[661], 12 += 1 per
killed bot) **[M]**.

**State-name matching in OS lists** (FUN_0044cb3c, FUN_0044cfda, FUN_0044cdf1) **[H]**: only
the *first letter* (and second for M) of each name counts: `A`→Armor, `E`→Experience,
`L`→Life, `MA`→Magic, `MO`→Money. Any other initial **stops parsing the rest of the list**.
Each name is followed (after any non-digit chars) by a signed decimal value. Only states 1–5
are reachable from these commands.

**Target prefix** (FUN_0040c0aa) **[H]**: if the list contains `=`, the first word may be `O…`
(apply to the opponent object, second table set) or `P…` (player, default).

| command | syntax | semantics |
|---|---|---|
| `SETOS` | `SETOS=(L=100,MA=0,A=50,MO=100,E=0)` or *`SETOS=a b c d e`* | Assign the listed states directly (no clamping, no tags). Positional form fills Life..Experience. [H] |
| `INCOS` | `INCOS(L=5)` | Add each value through the threshold logic (§7) → may clamp and fire tags. Missing states add 0. Positional *`INCOS a b c d e`* also accepted. [H] |
| `SUMOS` | `SUMOS(-5,A,L)` | Distribute one amount over the listed states in order. Negative (damage): each state gives up to its current value (it is not limited by the min table), remainder goes to the next state – armour absorbs first, then life. Deltas then go through the threshold logic (so LIFE→0 fires 661). Quirk: a positive amount always lands in the Life delta slot; the remainder of the `L` step is not carried on. [H] |
| `INCTOOS` | `INCTOOS(L=100,MA=0,A=50,MO=0,E=0)` | "Raise to": for each value > 0, if the state is lower, set it to the value (no tags). Zeros are ignored. [H] |
| `SETMAXOS` / `SETMINOS` | `SETMAXOS=(L=100,…)` | Set the per-state max / min table (player tables 0x510270 / 0x5102F0; opponent 0x5104B0 / 0x510530). [H] |
| `SETHIGHOS` / `SETLOWOS` | same | Set the "high" / "low" warning thresholds (0x5103F0 / 0x510370). [H] |
| `SETMAXOSTAGS`, `SETMINOSTAGS`, `SETHIGHOSTAGS`, `SETLOWOSTAGS` | `SETLOWOSTAGS=(L=671,MA=(672),A=(673),MO=680)` | Tag per state for each threshold. Value may be `n`, `(n)` or `a-b` (random tag in range when fired); stored as two 16-bit halves; ≤0 ignored. [H] |
| `STATEVAL` | `STATEVAL(11 + 1)` | `index op value` with op one of `= + - * /` on player state[index] (any index 0..15), raw, no clamping/tags. [H] |
| *`OMAXOS`, `OSETOS`, `OINCOS`, `OINCTOOS`* | | console variants that build an effect record and call the effect handler. |
| `XCHANGE` | `XCHANGE(MONEY=25,501,LIFE=20,502[,fulltag])` | Trade on the player: `give=amount,failTag,get=amount,okTag,fullTag`. If *give* is a full state name (case-insensitive "Life", "Magic", … – not the 1-letter form): fail (tag or "Sorry, you need more X") if state < amount; fail (fullTag or "You can't carry any more X") if *get* state ≥ its max; else subtract, add *get* (through thresholds) and fire okTag (or "Thank you, come again!"). If *give* is not a state it is an inventory item name: needs `amount` items, removes them. [H] Note: written inside mover code, `Experience=20` is reduced to `E` by pre-processing and is then **not** a state name → XCHANGE8 takes money and gives nothing [M]. |

### 6.2 Tags, messages, sound, text

| command | syntax | semantics |
|---|---|---|
| `TAG` | `TAG=600` | ActivateTag(n). [H] |
| `XT` | `XT(401)`, `XT(250-258)` | ActivateTag(n) / random tag in range; tag ≤0 ignored. [H] |
| `ONTAG` | `ONTAG[n]={ cmd(...) cmd(...) }` | Store the body (text after `=`) for tag n, 1 ≤ n ≤ 999 (only <999 can ever run). Re-definition replaces the old body. Executed by ActivateTag as a §3 command line. [H] |
| `XM` | `XM(199, "TEXT")`, `XM("TEXT")` | Show a status-line message in palette colour n (default 92 = 0x5C when omitted/0). FUN_00453951 + FUN_004b15c2. [H] |
| `OTEXT` | `OTEXT("text")` / *`OTEXT(obj "text")`* | Put text (≤126 chars) into a text object and show it; without an object name uses the default text object `_DEF_OT_` (created on demand). [H] |
| `OSOUND` | `OSOUND(NAME)` | If an object named NAME exists it is used, otherwise play the sound NAME (log "xcmd OSOUND sound doesn't exist" if missing). [H] |
| *`NOSEEUM`* | | toggles DAT_0051021c. |

### 6.3 Movers, tools, player physics

| command | syntax | semantics |
|---|---|---|
| `INITMOVES` | `INITMOVES=(DETHBOT1,…)` | For each name: attach the `[moveractioncodes]` script of the same name to the object of the same name (builds an OSETMOVE record with both names and calls the OSETMOVE effect). [H] |
| *`OSETMOVE`* | `OSETMOVE(object,script,confineregion,oncreatescript,flags,viewcone)` | explicit form (`%10s %10s %10s %10s %d %d`). [H] |
| `INITPARTSETS` | `INITPARTSETS=(names…)` | init particle set for each named object (FUN_004c0abd(name,3)). [M] |
| `AVAILTOOLS` | `AVAILTOOLS=000003FF` | hex bitmask of tools the player may select (bits 0–9). [H] |
| `OPTOOL[n]`, `OPPROJ[n]`, `OPXPLD[n]` | `OPTOOL[0]=( TN=0 TEX=TOUCH TY=01100030 FL=… FL2=… AMF=… XF=… R=16 H=32 MS=100 MX=(a b c d) AC=(a b c d) SNDS=(s1 s2 s3 s4 s5) NUMS=(n1..n5) )` | Define player tool n (0–15), its projectile and its explosion. Creates an object: TN team (+0x1AE), TEX texture, TY object type (+0), FL/FL2 flags, AMF (+0x1D0), XF, R radius, H height, MS, MX/AC four 16.16 values; SNDS five sound names (`~` = none) and NUMS five ints go into the aux record (+0x0E… / +0x88…). `*NOAUX*` = no aux part. Exact field format string at 0x51A26F. [H for parsing, L for meaning of MX/AC/NUMS] |
| `OOTOOL[n]`, `OOPROJ[n]`, `OOXPLD[n]` | same | same for "other" (bot) tools. [M] |
| `OTOOLCON[n]` | `OTOOLCON[8]=( MAGIC, 10, -5, 674)` | Must follow OPTOOL[n]. Tool control: needs ≥10 MAGIC, applies −5 per use, fires 674 if not enough (default text "You need more magic for…"). Effect handler FUN_00466112. [M] |
| `OSETJUMP` | `OSETJUMP=0,200,0,200,6,GRUNT03,-1,GRUNT01,BREAK01` | Player jump controller: five ints → controller +0x218/+0x21C/+0x220/+0x224 (16.16; `-1` = keep) and count +0x228 (6); four sound names (`-1` = none). Meaning of the four speeds/heights not resolved (LIPS03 uses 250). [H storage, L meaning] |
| *`INITTOOL[n]`, `TOOLSOUT`* | | tool init / debug dump (`AVAILTOOLS %08X`). |
| *`OPPONENT(name)`* | | sets the "opponent" object used by `O`-prefixed OS lists. [H] |
| *`WTX/WALLTX`, `WTY/WALLTY`, `WFADE/WALLFADE`, `WTEX/WALLTEX`, `FTEX`, `CTEX`, `FFADE`, `CFADE`* | | wall/floor/ceiling texture tweaks. |

The rest of the table (`FLYON` … `OPEN`, `MAIL*`, `KEYMAP`, `NETTAG`, …, `WEBCHAT*`, `RPORTAL`,
`LSETTINT`, `LSETFADE`) are console commands also accepted in INI command sections; none are
used by Lips.

### 6.4 Full command table (0x0050FA48, in table order)
`FLYON FLYOFF GOTO PLAY STOP PAUSE SOUNDON SOUNDOFF PHYSICSON PHYSICSOFF SET TRACE TAG WATCH
DOORBELL SHOW INVERTMOUSE SYSINFO CHECKWDIST NEW SPEAK JOIN NAME QUIT LOG MAIL MAILBUG
MAILCOMMENT MAILTO MAILFROM MAILSUBJECT KICKBAN GAMEPASSWORD NETTAG KEYMAP KEYBIND EXECUTE OPEN
OSETJUMP OSETCROU OSETZSPD OSETFLY OPRSTON ODRSTON OPHYSON OSETPOS OSETSPD OMAXOS OSETOS OINCOS
OINCTOOS OENGINE OSETGEN OSETVIEW OSETVIS OSTEPON OSETKEYS RPORTAL LSETTINT LSETFADE WEBCHATNEW
WEBCHATOFF SETMINOS SETMINOSTAGS SETMAXOS SETMAXOSTAGS SETLOWOS SETLOWOSTAGS SETHIGHOS
SETHIGHOSTAGS ONTAG OTEXT OSOUND OSETMOVE INITMOVES INITPARTSETS INCOS SUMOS INCTOOS SETOS
XCHANGE INITTOOL TOOLSOUT OPTOOL OTOOLCON OPPROJ OPXPLD OOTOOL OOPROJ OOXPLD AVAILTOOLS OPPONENT
XM XT NOSEEUM STATEVAL WTX WALLTX WTY WALLTY WFADE WALLFADE WTEX WALLTEX FTEX FFADE CTEX CFADE`
(INCOS=OINCOS, SETOS=OSETOS, INCTOOS=OINCTOOS share handlers.) Sub-keywords for SHOW/TRACE etc.
(table 0x0050FDB0): START DOMAIN PLAYER NETPLAYER LOCALPLAYER ORB CD AVI TRACE GROUP(S) OFF ON
WALLDIST RENDERFLAGS FLYHEIGHT URL MOVER(S) ACTIVES BUDDY SOUNDDIST SCRRES RENDRES FLYDOG
VIEWANGLE OPFRAMERATE BUG COMMENT POINT PMAP TTCOLOR NOSEEUM.

## 7. Threshold logic for player state changes (FUN_0044b9ee) **[H]**

`change(state i, delta)` on the player (opponent uses its own table set, other objects just add):

* `delta < 0`: nothing happens unless `cur > min[i]`. If `cur+delta ≤ min[i]` → state = min,
  fire minTag[i]. Else if `cur > low[i]` and `cur+delta ≤ low[i]` → apply, fire lowTag[i].
  Else apply.
* `delta ≥ 0`: nothing happens unless `cur < max[i]`. If `cur+delta ≥ max[i]` → state = max,
  fire maxTag[i]. Else if `cur < high[i]` and `cur+delta ≥ high[i]` → apply, fire highTag[i].
  Else apply.
* All tables start at 0 → before `SETMAXOS`, positive INCOS does nothing.
* A threshold tag fires only on the crossing step (edge-triggered). Tag value packs
  `lo | hi<<16`; if `lo < hi` a random tag in [lo,hi] fires (FUN_0044b0a6).

Shipped values (every level): max L100 MA100 A100 MO999999 E999999, min all 0, low L25 MA30
A10 MO0, high E300; tags max E685, min L661 MA662 A663 MO664 E665, low L671 MA672 A673 MO680,
high E675.

## 8. Named messages and the menu system

### 8.1 `[namedmessages]` table (FUN_0046e755 / lookup FUN_0046e93e) **[H]**
Up to 1024 entries of 16 bytes: key (upper-cased, truncated to 11 chars) + pointer to value.
Lines starting `;` or `/` are skipped; one leading `"` and one trailing `"` are stripped from the
value; empty values store no string. Lookup upper-cases the requested key, compares the first
11 chars. Used by OMENU/OHOTSPOT/GAME* effects (aux name fields), "Loading…" texts, XCHANGE
custom messages, etc.

### 8.2 Menu objects (XRB) **[H for parsing, M for layout]**
Menus are XRB objects; the INI only supplies strings. In LIPSTART.XRB:
* `MENU1`, `MENU2`, `MENUQUIT`: effect **OMENU** (effect index 85), tags 600 / 610 / 620. Aux
  record fields: +0x4A title key (`M1TITLE`), +0x56 subtitle key (`M1SUBTL`), +0x62 info key
  (`M1INFO`), +0x6E cursor object (`MENUCURS`), +0x7A hotspot prefix (`M1HS`), +0x88 input mode
  (3 = both keyboard and mouse handlers, 1 = FUN_00441513, 2 = FUN_004416aa), +0x8C/+0x90 title x/y
  (20,10), +0x94 title colour (199; default 199), +0x98 title background (−1 = none).
* On creation (event 0x2000000) OMENU collects objects named `<prefix>1`, `<prefix>2`, … until
  one is missing (`%s%d`) → `M1HS1..M1HS7`.
* When activated it parses the info string with `%d,%d,%d,%d,%d,%d,%d,%d` into
  **x, y, w, h, tl_y, il_y, bdr_clr, bak_clr**, sets the global "active menu" pointer and resets
  the selection.
* Drawing (FUN_00441c6f, called from FUN_00456aa4): box at (x,y) with width
  `max(8*len(longest of title/subtitle)+6, w)` and height h, filled with bak_clr if > 0; title
  at (titleX, titleY) and subtitle 12 px lower in the title colour; if the info string was
  parsed: border rectangle in bdr_clr, and if tl_y > 0 a horizontal rule below the
  title block, if il_y > 0 another rule at y = il_y.
* Hotspots (`M1HSn`, effect **OHOTSPOT** 84): aux +0x4A message key (`M1HS1`), +0x88/+0x8C
  position, +0x90 save-slot number, +0x94 tag fired on selection, +0x3E sound. Object +0x54 =
  item kind: 1 text item (string from the key), 4 save-slot item (shows the character name of
  slot +0x90 if that slot is active, else falls back to the key text, i.e. `---`), 0 picture
  item (sprite, text only when selected). Selected item colour `DAT_00510728`, others
  `DAT_00510724`.
* Main-menu flow (LIPSTART, from aux tags): `New Game`→601 GAMENEW (asks "Enter Character
  Name:", ok→610 menu 2, fail→600); slot 1–5 → 602 GAMECUR (active slot→610, empty→601);
  `Quit`→629 OQUIT. Menu 2: `Start Game Now!`→619 GAMEGOTO (orb from slot, else aux orb
  `LIPS00`), `Copy`→612 GAMECOPY, `Delete`→613 GAMEDEL (confirm by pressing Y), `Go to Game
  Menu`→600, `Quit`→629. Quit menu (level INIs): Save→621 GAMESAVE (→620), Continue,
  Game-menu (GAMEGOTO with object kind 1 → back to `startgameorbname`), Quit→629.

### 8.3 GAME* effects (save slots, see re/formats/GAM.md) **[H]**
| effect | behaviour |
|---|---|
| GAMENEW | find a free slot (1–5); none → "No Available Game Slots" + fail tag (+0x98); else prompt "Enter Character Name:", create slot (flags 1\|2), write file, select it, success tag (+0x94) |
| GAMECUR | select slot from the triggering hotspot's +0x90 (or aux +0x90); active → +0x94 tag + "<name>'s game Selected"; empty → +0x98 + "Open new game selected" |
| GAMECOPY | prompt "Character Name:", copy current slot into a free slot |
| GAMEDEL | confirmation "Confirm: touch Y to Delete"; on Y clears the slot and rewrites the file |
| GAMESAVE | snapshot player into current slot (flags \|= 4), write file, "Game saved for <name>", then +0x94 tag (or quit / return depending on object kind) |
| GAMEGOTO | load the slot's orb (or the aux orb if slot never saved), copy its states into the player, "Going to game for <name>" |
| GAMEQUIT | = OQUIT |

## 9. `[moveractioncodes]` (bot scripts) – overview **[M/L]**

Syntax: `NAME = [IN { cmds }] [ON <cond> { cmds }]×≤4 body … *` (pre-processed like §2, so
`SetHitstokill` → `SH`). Compiled by FUN_0044a3f8 (IN block FUN_00444ba0, ON blocks
FUN_004450f8, condition letters FUN_00444dc8/FUN_00444cef), interpreted by FUN_004454b9,
FUN_004457d3, FUN_004470f2 … ; up to 199 scripts. An `ONTAG[` line inside this section is
treated like the command (FUN_0044a4c6).
* ON conditions are letter pairs: event letter `B` bump, `H` hit, `C`, `V`, `E`, `L`, `X`,
  plus `F(n)`, `M(n)`, `S(n)`, `T(tag)` (tag condition; marks the tag as referenced), `U"text"`;
  the second letter is the actor: `P` player, `O` object, `N` net player, `A` any, `K` kill
  (hit that kills), `W` wall, `C`, `F`. Shipped: `ON BP` (bumped by player), `ON HP` (hit by
  player), `ON HK` (killed).
* Body/opcodes used (meaning inferred from use): `IN{SH(n)}` hits-to-kill, `TP` target player,
  `F(n)` face/follow, `P(n,tag)` attack with tag on hit (`P(1,411)`), `W(n)` walk/wait, `MD(n)`,
  `U(n)`, `XL(name)` drop/launch object (`DROPBONZ`), `XN("name")` become/spawn named object,
  `XD` delete self, `XT(tag)`, `XM(col,"txt")`, `OTEXT`, `OSOUND`, `XCHANGE(...)`,
  `SV(IS10 + 1)` set state value, `WT(tag)` wait for tag, `PS(snd,n)` play sound, `PTL(a,b)`,
  `*` end of script. Missing `\` in XCHANGE7 (shipped) ends that script after its ON BP block.

## 10. XRB ↔ tag references (for the reimplementation) **[H]**
* Object record (480 B): +0x44 tag (16.16, −1 = none), +0x140 effect index into the 274-entry
  effect table (registration order of FUN_004727f4), +0x1B0 aux index, +0x54 item kind,
  +0x10C hotspot order.
* Aux record (176 B): +0x0E..+0x7A 12-byte name slots (sound +0x3E, message key +0x4A, etc.),
  +0x88..+0x98 five ints (positions/slot/success tag/fail tag), +0x9C..+0xAC sound handles.
* Walls: tag +0xA4, effect +0xD0. Regions: tag +0x84, effect +0xB0.

## 11. How menu / status text is drawn (and why it is invisible under Wine) **[H]**

* All in-game text (menu title/subtitle FUN_0046d0eb, hotspot labels FUN_00475bb4, status
  messages, OTEXT overlays) is software-rendered into the 8-bit frame buffer with an **8×8
  monochrome bitmap font**; one glyph = 8 bytes, MSB = leftmost pixel, only set bits are
  written (transparent background unless a background colour ≥ 0 is given, in which case a
  `8*len+6` × 12 box is filled first). Char advance 8 px. Byte 0xFE inside a string switches the
  colour to the next byte.
* The glyph address is computed as `char*8 + 0x000FFA6E`, i.e. real-mode **F000:FA6E – the IBM
  PC BIOS ROM 8×8 character font**. Five glyph blitters use this constant: FUN_0046ce0f,
  FUN_0047550a, FUN_00475ab1, FUN_00475d7f, FUN_00475eda (verified in the disassembly:
  `shl eax,3 / add eax,0xFFA6E` at 0x475B19). There is no font in the game data.
* This works on Windows 9x because the first megabyte (V86/DOS arena incl. the BIOS ROM) is
  mapped readable into every Win32 process. Under Wine (and NT) that range is not the BIOS: on
  the Xvfb/Wine run it is evidently mapped but all zero (no crash), so every glyph test fails and
  nothing is drawn – the menu boxes and rules (plain fills) still appear. That is exactly the
  observed symptom.
* GDI text (`DrawTextA`/`TextOutA`) is used only for the announcement pop-up window
  (FUN_004b06b0/FUN_004b0fea, e.g. "New game for …") and one window-text routine (TextOutA in FUN_004ba2f0, not the menu).
* Chars ≥ 0x80 index past the ROM font (F000:FE6E…, BIOS code) → garbage on real hardware too.
* Reimplementation: embed the standard IBM 8×8 CP437 font (first 128 glyphs). For testing the
  original under Wine, either commit/write the 1 KB font at linear 0x000FFA6E in the process
  (e.g. launcher with CreateProcess(SUSPENDED) + VirtualAllocEx/WriteProcessMemory) or patch the
  five `add eax,0xFFA6E` immediates to point at a font copy.
