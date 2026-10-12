"""Dump XRB objects grouped by effect (OTHING, OTELE, ...) with their aux record, for checking
the object-effect handler interpretation (re/engine/object-effects.md).

usage:
  python3 -I re/scripts/effect_dump.py [--dir original] [--level LIPS03] [--max N] [--raw] EFFECT...
  python3 -I re/scripts/effect_dump.py --counts          # usage count per effect over all levels

Per object it prints: level, object index, name, flags (+0), +0x0c/+0x10 flags, tag (+0x44 int part),
+0x54 counter/kind, effect name, a few params, and its aux record: the 10 name slots (+0x0e+12k),
the five ints (+0x88..+0x98) and the five resolved Sound names (+0x9c..+0xac).
--raw adds a word dump of the non-zero object fields.
"""
import os
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xrb_lib import Xrb, i32, u32, i16, u16  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))


def effect_names():
    """Registration order of the effect table = row order in re/engine/effects.md."""
    names = []
    for line in open(os.path.join(ROOT, "re", "engine", "effects.md"), encoding="utf-8"):
        m = re.match(r"\| (\S[^|]*?) \| (\S+) \| (0x[0-9a-f]+|\d+) \|", line)
        if m and m.group(1) != "name":
            names.append(m.group(1))
    return names


EFF = effect_names()


def cstr(b):
    return b.split(b"\0")[0].decode("latin1")


def obj_name(r):
    return cstr(r[0x58:0x60])


def aux_info(x, idx):
    if idx < 0 or idx >= x.count("Auxs"):
        return None
    a = x.rec("Auxs", idx)
    slots = [cstr(a[0x0e + 12 * k:0x0e + 12 * k + 12]) for k in range(10)]
    ints = [i32(a, 0x88 + 4 * k) for k in range(5)]
    snds = []
    for k in range(5):
        s = i32(a, 0x9c + 4 * k)
        snds.append(cstr(x.rec("Sounds", s)[0x0c:0x24]) if 0 <= s < x.count("Sounds") else None)
    return slots, ints, snds


def fmt_aux(info):
    if info is None:
        return "aux=-"
    slots, ints, snds = info
    sl = " ".join(f"s{k}={v!r}" for k, v in enumerate(slots) if v.strip())
    sn = " ".join(f"S{k}={v}" for k, v in enumerate(snds) if v)
    return f"aux ints={ints} {sl} {sn}".rstrip()


def levels(d, only=None):
    for f in sorted(os.listdir(d)):
        if f.upper().endswith(".XRB") and (only is None or f.upper().startswith(only.upper())):
            yield f[:-4], Xrb(os.path.join(d, f))


def by_name(x):
    return {obj_name(r): i for i, r in enumerate(x.recs("Objects"))}


def main(argv):
    d = os.path.join(ROOT, "original")
    only = None
    mx = 10 ** 9
    raw = False
    counts = False
    wanted = []
    it = iter(argv)
    for a in it:
        if a == "--dir":
            d = next(it)
        elif a == "--level":
            only = next(it)
        elif a == "--max":
            mx = int(next(it))
        elif a == "--raw":
            raw = True
        elif a == "--counts":
            counts = True
        else:
            wanted.append(a.upper())
    if counts:
        c = {}
        for _, x in levels(d, only):
            for r in x.recs("Objects"):
                e = i32(r, 0x140)
                n = EFF[e] if 0 <= e < len(EFF) else str(e)
                c[n] = c.get(n, 0) + 1
        for n, k in sorted(c.items(), key=lambda t: -t[1]):
            print(f"{k:5d} {n}")
        return
    shown = {}
    for lv, x in levels(d, only):
        names = by_name(x)
        npts = x.count("Points")
        for i, r in enumerate(x.recs("Objects")):
            e = i32(r, 0x140)
            en = EFF[e] if 0 <= e < len(EFF) else str(e)
            if wanted and en not in wanted:
                continue
            if shown.get(en, 0) >= mx:
                continue
            shown[en] = shown.get(en, 0) + 1
            pt = i32(r, 0x14)
            pos = ""
            if 0 <= pt < npts:
                p = x.rec("Points", pt)
                pos = f"@({i32(p, 0x14) >> 16},{i32(p, 0x18) >> 16},{i32(p, 0x1c) >> 16})"
            tag = i32(r, 0x44)
            print(f"{lv} #{i:<4} {obj_name(r)!r:12} {en:8} fl={u32(r, 0):#010x} "
                  f"f0c={u32(r, 0x0c):#x} f10={u32(r, 0x10):#x} f1d0={u32(r, 0x1d0):#x} "
                  f"tag={tag >> 16 if tag != -1 else -1} k54={i32(r, 0x54)} "
                  f"r3c={i32(r, 0x3c) / 65536:g} h34={i32(r, 0x34) / 65536:g} ang={u16(r, 0x24)} "
                  f"reg={i32(r, 0xfc)} tex={i32(r, 0x64)} {pos}")
            refs = []
            for off, lab in [(0x18, "pt2"), (0x108, "wall"), (0x10c, "ovl"), (0x110, "path"),
                             (0x114, "win"), (0x144, "o144"), (0x148, "o148"), (0x150, "view"),
                             (0x174, "reg174"), (0x1a8, "text"), (0x1bc, "pt1bc"), (0x1c0, "pt1c0")]:
                v = i32(r, off)
                if v not in (-1,):
                    refs.append(f"{lab}={v}")
            for k in range(6):
                v = i32(r, 0x18c + 4 * k)
                if v != -1:
                    refs.append(f"o{0x18c + 4 * k:x}={v}")
            params = " ".join(f"{o:x}={i32(r, o)}" for o in range(0x7c, 0xac, 4) if i32(r, o))
            params2 = " ".join(f"{o:x}={i32(r, o)}" for o in range(0x118, 0x140, 4) if i32(r, o))
            print(f"      refs: {' '.join(refs)}  p7c: {params}  p118: {params2}")
            print(f"      {fmt_aux(aux_info(x, i32(r, 0x1b0)))}")
            if raw:
                words = struct.unpack_from("<120i", r)
                nz = [f"{4 * k:03x}:{w:#x}" for k, w in enumerate(words) if w not in (0,)]
                for k in range(0, len(nz), 8):
                    print("        " + " ".join(nz[k:k + 8]))


if __name__ == "__main__":
    main(sys.argv[1:])
