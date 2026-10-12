"""Dump the parameters of wall/region effects from the shipped levels and check them against
re/engine/world-effects.md.

usage:
  python3 -I re/scripts/wallregion_dump.py [DIR]                 # summary + plausibility checks
  python3 -I re/scripts/wallregion_dump.py [DIR] --list RTDOOR    # one line per wall/region
  python3 -I re/scripts/wallregion_dump.py [DIR] --anim           # texture-animation statistics

DIR defaults to ./original. Effect indices follow the registration order of re/engine/effects.md.
"""
import collections
import glob
import os
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xrb_lib import Xrb, i32, i16, u16, u32, fx  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))

# Names of the '&DAT' rows of effects.md, read from the strings in lips.exe.
# In registration order of the '&DAT' rows.
UNNAMED = ["OPAL", "ONEW", "OEND", "OKEY", "WTV", "WLEN", "WTOW", "RTV", "VDEF", "VROT", "VINS"]


def effect_names():
    out = []
    for line in open(os.path.join(HERE, "..", "engine", "effects.md")):
        m = re.match(r"\| (\S[^|]*?) \| (\S+) \| (\S+) \|", line)
        if m and m.group(1) not in ("name", "---") and not m.group(1).startswith("undefined"):
            out.append(m.group(1))
    it = iter(UNNAMED)
    return [next(it) if n == "&DAT" else n for n in out]


EFF = effect_names()
IDX = {n: i for i, n in enumerate(EFF)}


def cstr(b):
    return b.split(b"\0")[0].decode("latin1").strip()


class Level:
    def __init__(self, path):
        self.x = x = Xrb(path)
        self.name = os.path.basename(path)
        self.walls = x.recs("Walls")
        self.regions = x.recs("Regions")
        self.auxs = x.recs("Auxs")
        self.sounds = x.recs("Sounds")
        self.texs = x.recs("Texs")
        # per-tex data blocks (entries, pic table, mirror table), in Tex order
        self.texdata = []
        off = x.off["TexData"]
        for t in self.texs:
            size = u32(t, 0x50)
            self.texdata.append(x.body[off:off + size])
            off += size
        self.region_by_name = {}
        for k, r in enumerate(self.regions):
            self.region_by_name.setdefault(cstr(r[0xa4:0xac]), k)
        self.wall_by_name = {}
        for k, w in enumerate(self.walls):
            self.wall_by_name.setdefault(cstr(w[0xb0:0xb8]), k)

    # --- aux --------------------------------------------------------------------------------
    def aux(self, idx):
        if idx < 0 or idx >= len(self.auxs):
            return None
        a = self.auxs[idx]
        slots = [cstr(a[0x0e + 12 * k:0x0e + 12 * k + 12]) for k in range(10)]
        params = [i32(a, 0x88 + 4 * k) for k in range(5)]
        snds = [i32(a, 0x9c + 4 * k) for k in range(5)]
        return {"slots": slots, "p": params, "snd": [self.sound(s) for s in snds]}

    def sound(self, idx):
        if idx < 0 or idx >= len(self.sounds):
            return None
        return cstr(self.sounds[idx][0x0c:0x24])

    # --- textures ---------------------------------------------------------------------------
    def tex(self, idx):
        if idx < 0 or idx >= len(self.texs):
            return None
        t = self.texs[idx]
        n = u16(t, 0x1a)
        d = self.texdata[idx]
        entries = [struct.unpack_from("<HHiI", d, 12 * k)[:3] for k in range(n)]
        frames, rots = u16(t, 0x10), u16(t, 0x12)
        mirrors = d[12 * n + 4 * frames * rots:12 * n + 5 * frames * rots]
        starts = [u16(t, 0x24 + 2 * k) for k in range(16)]
        return {"type": u32(t, 0), "name": cstr(t[4:12]), "n": n, "entries": entries,
                "frames": frames, "rots": rots, "mirrors": bytes(mirrors), "starts": starts}

    def tc_desc(self, rec, off):
        t = self.tex(i32(rec, off))
        if t is None:
            return "-"
        if t["type"] & 1:
            return f"{t['name']}*{t['n']}"   # animated sequence
        return t["name"]


def aux_str(a):
    if a is None:
        return "aux=-"
    sl = ",".join(f"{k}:{s}" for k, s in enumerate(a["slots"]) if s)
    sn = ",".join(f"{k}:{s}" for k, s in enumerate(a["snd"]) if s)
    return f"p={a['p']} slots[{sl}] snd[{sn}]"


def region_line(L, k):
    r = L.regions[k]
    a = L.aux(i32(r, 0x10c))
    return (f"{L.name} R{k:<4} '{cstr(r[0xa4:0xac])}' tag={i16(r, 0x86)} "
            f"fl={fx(r, 0xc):g} ce={fx(r, 0x10):g} rest=({fx(r, 0x14):g},{fx(r, 0x18):g}) "
            f"A..E=({i32(r, 0x6c)},{i32(r, 0x70)},{i32(r, 0x74)},{i32(r, 0x78)},{i32(r, 0x7c)}) a0={i32(r, 0xa0)} "
            f"pt/obj=({i32(r, 0xf8)},{i32(r, 0xfc)}) "
            f"f={u32(r, 0):#x}/{u32(r, 4):#x} objs={'y' if i32(r, 0xb4) >= 0 else 'n'} "
            f"tex=({L.tc_desc(r, 0x1c)},{L.tc_desc(r, 0x34)}) {aux_str(a)}")


def wall_line(L, k):
    w = L.walls[k]
    a = L.aux(i32(w, 0xdc))
    return (f"{L.name} W{k:<5} '{cstr(w[0xb0:0xb8])}' tag={i16(w, 0xa6)} light={i16(w, 0xa4)} "
            f"f={u32(w, 0):#x}/{u32(w, 4):#x} xoff={fx(w, 0x9c):g} yoff={fx(w, 0xa0):g} "
            f"ac={i32(w, 0xac)} bc={i32(w, 0xbc)} back={'y' if i32(w, 0x84) >= 0 else 'n'} "
            f"tex=({L.tc_desc(w, 0x2c)},{L.tc_desc(w, 0x44)},{L.tc_desc(w, 0x5c)}) {aux_str(a)}")


def load(d):
    return [Level(p) for p in sorted(glob.glob(os.path.join(d, "*.XRB")))]


def by_effect(levels):
    walls = collections.defaultdict(list)
    regions = collections.defaultdict(list)
    for L in levels:
        for k, w in enumerate(L.walls):
            e = i32(w, 0xd0)
            if e >= 0:
                walls[EFF[e]].append((L, k))
        for k, r in enumerate(L.regions):
            e = i32(r, 0xb0)
            if e >= 0:
                regions[EFF[e]].append((L, k))
    return walls, regions


# ---------------------------------------------------------------------------------------------
def check(label, ok, total, note=""):
    print(f"  [{'ok' if ok == total else '!!'}] {label}: {ok}/{total} {note}")


def summary(levels):
    walls, regions = by_effect(levels)
    print("== usage")
    for kind, d in (("walls", walls), ("regions", regions)):
        print(" ", kind, {k: len(v) for k, v in sorted(d.items(), key=lambda kv: -len(kv[1]))})

    # ---- RTDOOR -----------------------------------------------------------------------------
    # The XRB is a snapshot taken after the editor ran each effect's init (event 1): the door is
    # stored closed (ceil == floor, or floor + tallest object + 3), var C (+0x74) holds the open
    # ceiling, +0xa0 already holds the 16.16 speed, region+4 bit 0x800000 = "closed" flag.
    print("\n== RTDOOR (C=+0x74 open ceiling; speed=+0xa0 16.16 u/1.2s; aux+0x98 = auto-close x100ms)")
    rows = regions["RTDOOR"]
    R = [L.regions[k] for L, k in rows]
    A = [L.aux(i32(L.regions[k], 0x10c)) for L, k in rows]
    print("  speed(+0xa0, u per 1.2 s):", dict(collections.Counter(fx(r, 0xa0) for r in R)))
    print("  aux+0x98 (auto-close delay, 100 ms units):",
          dict(collections.Counter(a["p"][4] if a else None for a in A)))
    print("  sounds (aux+0x9c start, +0xa0 loop, +0xa4 stop):",
          dict(collections.Counter(tuple(a["snd"][:3]) if a else None for a in A).most_common(6)))
    print("  open height C-floor:",
          dict(collections.Counter(fx(r, 0x74) - fx(r, 0xc) for r in R).most_common(10)))
    check("stored closed: ceil == floor", sum(i32(r, 0x10) == i32(r, 0xc) for r in R), len(R))
    check("C (+0x74) == rest ceiling (+0x18)", sum(i32(r, 0x74) == i32(r, 0x18) for r in R), len(R),
          "(misses = C equals floor: door that can never open)")
    check("rest floor (+0x14) == floor", sum(i32(r, 0x14) == i32(r, 0xc) for r in R), len(R))
    check("closed flag region+4 bit 0x800000", sum(bool(u32(r, 4) & 0x800000) for r in R), len(R))
    check("speed > 0 (first move opens)", sum(i32(r, 0xa0) > 0 for r in R), len(R))
    check("state D == 0 and timer B == 0", sum(i32(r, 0x78) == 0 and i32(r, 0x70) == 0 for r in R),
          len(R))
    check("has a tag (>0)", sum(i16(r, 0x86) > 0 for r in R), len(R))
    check("has an aux (sounds/wait)", sum(a is not None for a in A), len(R))
    for (L, k), r in zip(rows, R):
        if i32(r, 0x74) != i32(r, 0x18) or i16(r, 0x86) <= 0:
            print("     odd:", region_line(L, k)[:160])

    # ---- RFUP1 ------------------------------------------------------------------------------
    print("\n== RFUP1 (C = bottom floor; floor oscillates C <-> ceil - tallest object - 2 at +0xa0)")
    for L, k in regions["RFUP1"]:
        r = L.regions[k]
        print("  ", region_line(L, k)[:150])
    check("C (+0x74) == floor, D == -1",
          sum(i32(L.regions[k], 0x74) == i32(L.regions[k], 0xc) and i32(L.regions[k], 0x78) == -1
              for L, k in regions["RFUP1"]), len(regions["RFUP1"]))

    # ---- RFBOUNCE ---------------------------------------------------------------------------
    print("\n== RFBOUNCE (jump pad: A = B = aux+0x88 << 16 bounce vz, C = aux+0x8c mode)")
    c = collections.Counter()
    for L, k in regions["RFBOUNCE"]:
        a = L.aux(i32(L.regions[k], 0x10c))
        c[(tuple(a["p"]) if a else None, a["snd"][4] if a else None)] += 1
    for kv, n in c.most_common():
        print(f"   {n:3}x aux params {kv[0]} bounce-sound(aux+0xac)={kv[1]}")
    rows = regions["RFBOUNCE"]
    check("region flag 0x200 (jump pad) set", sum(bool(u32(L.regions[k], 0) & 0x200) for L, k in rows),
          len(rows))
    check("A == aux+0x88 << 16", sum(i32(L.regions[k], 0x6c) ==
                                     (L.aux(i32(L.regions[k], 0x10c)) or {"p": [0]})["p"][0] << 16
                                     for L, k in rows), len(rows))

    # ---- RROTATE / RMOVE -------------------------------------------------------------------
    objs_by_level = {}
    for e in ("RROTATE", "RMOVE"):
        print(f"\n== {e}  (RROTATE: B=mode, +0x7c=speed/frame, +0xf8=pivot point; RMOVE: slot 9 = carrier)")
        for L, k in regions[e]:
            r = L.regions[k]
            print("  ", region_line(L, k)[:200])
            a = L.aux(i32(r, 0x10c))
            if L.name not in objs_by_level:
                objs_by_level[L.name] = [cstr(o[0x58:0x60]) for o in L.x.recs("Objects")]
            if a and a["slots"][9]:
                piv = a["slots"][9]
                print(f"      slot9 '{piv}': object={'yes' if piv in objs_by_level[L.name] else 'no'}")
            if e == "RROTATE":
                ok = i32(r, 0x70) == (a["p"][0] if a else i32(r, 0x70)) and \
                    i32(r, 0x7c) == (a["p"][1] if a else i32(r, 0x7c))
                print(f"      B==aux+0x88 and +0x7c==aux+0x8c: {ok}; pivot point index {i32(r, 0xf8)}")
            else:
                ws = [i32(L.walls[i32(L.x.body, L.x.off['Trailer'] + 4 * (i32(r, 0x110) + j))], 0x88)
                      for j in range(i32(r, 0xbc))]
                print(f"      walls whose +0x88 == this region: {sum(w == k for w in ws)}/{len(ws)}")

    # ---- RSLANT -----------------------------------------------------------------------------
    print("\n== RSLANT (slots 5/6 = high/low region, 7/8 = edge walls; aux+0x90/0x94 fallback heights)")
    for L, k in regions["RSLANT"]:
        r = L.regions[k]
        a = L.aux(i32(r, 0x10c))
        print("  ", region_line(L, k))
        if not a:
            continue
        hi, lo = a["slots"][5], a["slots"][6]
        hr, lr = L.region_by_name.get(hi), L.region_by_name.get(lo)
        hz = fx(L.regions[hr], 0xc) if hr is not None else None
        lz = fx(L.regions[lr], 0xc) if lr is not None else None
        w1, w2 = L.wall_by_name.get(a["slots"][7]), L.wall_by_name.get(a["slots"][8])
        # contained regions (bbox inside, floor >= slant floor, ceil <= slant ceil)
        inner = []
        for j, o in enumerate(L.regions):
            if j == k:
                continue
            if (i32(o, 0xc4) >= i32(r, 0xc4) and i32(o, 0xc8) <= i32(r, 0xc8) and
                    i32(o, 0xcc) >= i32(r, 0xcc) and i32(o, 0xd0) <= i32(r, 0xd0) and
                    i32(o, 0xc) >= i32(r, 0xc) and i32(o, 0x10) <= i32(r, 0x10)):
                inner.append(j)
        print(f"      high='{hi}'->{hr} z={hz}  low='{lo}'->{lr} z={lz}  "
              f"edge walls {a['slots'][7]}->{w1} {a['slots'][8]}->{w2}  contained regions={len(inner)}")

    # ---- RANIM / WANIM / WTEXHOR -------------------------------------------------------------
    print("\n== RANIM / WANIM: are the textures animated sequences (Tex type bit 0)?")
    for e, lst, offs in (("RANIM", regions["RANIM"], (0x1c, 0x34)),
                         ("WANIM", walls["WANIM"], (0x2c, 0x44, 0x5c))):
        anim = 0
        for L, k in lst:
            rec = (L.regions if e == "RANIM" else L.walls)[k]
            if any((L.tex(i32(rec, o)) or {"type": 0})["type"] & 1 for o in offs):
                anim += 1
        print(f"  {e} with at least one animated slot: {anim}/{len(lst)} (rest are static: harmless)")

    print("\n== WTEXHOR (speed = aux+0x8c & 0xff texels/1.2 s; aux+0x88 != 0 -> negative)")
    for L, k in walls["WTEXHOR"]:
        print("  ", wall_line(L, k))

    print("\n== WTV (aux slot 9 = view name; aux+0x88 = view on at start)")
    for L, k in walls["WTV"]:
        print("  ", wall_line(L, k))
        a = L.aux(i32(L.walls[k], 0xdc))
        if a:
            views = [cstr(v[0x5c:0x64]) for v in L.x.recs("Views")]
            print(f"      view '{a['slots'][9]}' exists: {a['slots'][9] in views}")

    # ---- WTAGSW / RTAGSW -----------------------------------------------------------------------
    print("\n== WTAGSW")
    rows = walls["WTAGSW"]
    check("has tag > 0", sum(i16(L.walls[k], 0xa6) > 0 for L, k in rows), len(rows))
    W = [L.walls[k] for L, k in rows]
    check("init flag (byte1 0x20 = flags 0x2000) present", sum(bool(u32(w, 0) & 0x2000) for w in W),
          len(W), "(XRB = post-init snapshot)")
    sb = sum(1 for w in W if u32(w, 0) & 2 and u32(w, 0) & 1 and i16(w, 0xa6) >= 0)
    sn = sum(1 for w in W if u32(w, 0) & 2 and not u32(w, 0) & 1)
    pp = sum(1 for w in W if not u32(w, 0) & 2)
    print(f"  solid + flag 0x1 + tag>=0 (early 0x200 in the solid branch): {sb}")
    print(f"  solid without flag 0x1 (0x200 from the common blocked/contact exit): {sn}")
    print(f"  passable portals (0x200 on contact before crossing, see world-effects.md): {pp}")
    print("  aux+0x88 (tag range end):", dict(collections.Counter(
        (L.aux(i32(L.walls[k], 0xdc)) or {"p": [None]})["p"][0] for L, k in rows).most_common(8)))
    print("  aux+0x98 (activator filter):", dict(collections.Counter(
        (L.aux(i32(L.walls[k], 0xdc)) or {"p": [None] * 5})["p"][4] for L, k in rows).most_common(8)))
    print("  limited uses (flags byte1 & 0x10) count +0xac:", dict(collections.Counter(
        i32(L.walls[k], 0xac) for L, k in rows if u32(L.walls[k], 0) & 0x1000).most_common(8)))
    two = 0
    for L, k in rows:
        w = L.walls[k]
        for o in (0x2c, 0x44, 0x5c):
            t = L.tex(i32(w, o))
            if t and t["type"] & 1 and t["starts"][1] != t["starts"][0]:
                two += 1
                break
    print(f"  walls with a texture having a distinct action-1 sequence (switch on/off look): {two}")

    print("\n== RTAGSW (enter -> aux+0x88, leave -> +0x8c, land -> +0x90, ceiling -> +0x94)")
    rows = regions["RTAGSW"]
    c = collections.Counter()
    for L, k in rows:
        a = L.aux(i32(L.regions[k], 0x10c))
        if a is None:
            c["no aux"] += 1
            continue
        p = a["p"]
        key = "+".join(n for n, v in zip(("enter", "leave", "land", "ceil"), p[:4]) if v > 0)
        c[(key or "none", p[4])] += 1
    for kv, n in c.most_common(12):
        print(f"   {n:4}x {kv}")
    print("  +0xa0 (use count):", dict(collections.Counter(i32(L.regions[k], 0xa0) for L, k in rows)))


def anim_stats(levels):
    """Delay/next statistics of the Tex sequences used by walls and regions."""
    delays = collections.Counter()
    nexts = collections.Counter()
    loops = collections.Counter()
    for L in levels:
        used = set()
        for w in L.walls:
            used.update(i32(w, o) for o in (0x2c, 0x44, 0x5c))
        for r in L.regions:
            used.update(i32(r, o) for o in (0x1c, 0x34))
        for t in used:
            d = L.tex(t)
            if not d or not d["type"] & 1:
                continue
            for fb, nx, dl in d["entries"]:
                delays[dl / 65536] += 1
                nexts["98 hold" if nx == 98 else "99 end" if nx == 99 else
                      "in range" if nx < d["n"] else "out of range"] += 1
            loops[(d["name"], d["n"], tuple(d["entries"][0][2:3]))] += 1
    print("world (wall/region) animated texture entries: delay (16.16 -> units) histogram")
    for k, v in sorted(delays.items()):
        print(f"   delay {k:g}: {v}")
    print("next field:", dict(nexts))


def main(argv):
    d = "original"
    args = list(argv)
    if args and not args[0].startswith("--"):
        d = args.pop(0)
    levels = load(d)
    if not args:
        summary(levels)
    elif args[0] == "--anim":
        anim_stats(levels)
    elif args[0] == "--list":
        name = args[1].upper()
        walls, regions = by_effect(levels)
        for L, k in walls.get(name, []):
            print(wall_line(L, k))
        for L, k in regions.get(name, []):
            print(region_line(L, k))
    else:
        print(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
