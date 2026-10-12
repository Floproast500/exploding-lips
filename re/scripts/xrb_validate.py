"""Validate the XRB record-layout hypotheses (re/formats/XRB-records.md) against every level.

usage: python3 -I re/scripts/xrb_validate.py [files...]   (default: original/*.XRB)
Prints one line per check per file plus a totals table; exit code 1 if any hard check fails.
"""
import glob, math, os, struct, sys
from collections import Counter, defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xrb_lib import Xrb, i32, u32, u16, i16, fx  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# Index fields resolved by the post-load fix-up (SetOrbPointers): (section, offset, target)
REFS = [
    ("Points", 0x3c, "Points"), ("Points", 0x40, "Paths"),
    ("Regions", 0x1c, "Texs"), ("Regions", 0x24, "Pics"), ("Regions", 0x34, "Texs"),
    ("Regions", 0x3c, "Pics"), ("Regions", 0x5c, "Regions"), ("Regions", 0x60, "Regions"),
    ("Regions", 0x64, "Regions"), ("Regions", 0x68, "Regions"), ("Regions", 0xb4, "Objects"),
    ("Regions", 0xf4, "Objects"), ("Regions", 0xf8, "Points"), ("Regions", 0xfc, "Objects"),
    ("Regions", 0x100, "Sounds"), ("Regions", 0x10c, "Auxs"),
    ("Walls", 0x0c, "Points"), ("Walls", 0x10, "Points"), ("Walls", 0x14, "Points"),
    ("Walls", 0x18, "Points"), ("Walls", 0x1c, "Points"), ("Walls", 0x20, "Points"),
    ("Walls", 0x24, "Points"), ("Walls", 0x28, "Points"),
    ("Walls", 0x2c, "Texs"), ("Walls", 0x34, "Pics"), ("Walls", 0x44, "Texs"),
    ("Walls", 0x4c, "Pics"), ("Walls", 0x5c, "Texs"), ("Walls", 0x64, "Pics"),
    ("Walls", 0x80, "Regions"), ("Walls", 0x84, "Regions"), ("Walls", 0x88, "Regions"),
    ("Walls", 0xdc, "Auxs"),
    ("Objects", 0x14, "Points"), ("Objects", 0x18, "Points"), ("Objects", 0x64, "Texs"),
    ("Objects", 0x6c, "Pics"), ("Objects", 0xfc, "Regions"), ("Objects", 0x100, "Regions"),
    ("Objects", 0x104, "Regions"), ("Objects", 0x108, "Walls"), ("Objects", 0x10c, "Overlays"),
    ("Objects", 0x110, "Paths"), ("Objects", 0x114, "Windows"),
    ("Objects", 0x144, "Objects"), ("Objects", 0x148, "Objects"), ("Objects", 0x14c, "Objects"),
    ("Objects", 0x150, "Views"), ("Objects", 0x174, "Regions"),
    ("Objects", 0x18c, "Objects"), ("Objects", 0x190, "Objects"), ("Objects", 0x194, "Objects"),
    ("Objects", 0x198, "Objects"), ("Objects", 0x19c, "Objects"), ("Objects", 0x1a0, "Objects"),
    ("Objects", 0x1a8, "Texts"), ("Objects", 0x1b0, "Auxs"), ("Objects", 0x1b4, "ObjectStates"),
    ("Objects", 0x1bc, "Points"), ("Objects", 0x1c0, "Points"),
    ("Views", 0x2c, "Objects"), ("Views", 0xac, "Auxs"),
    ("Auxs", 0x9c, "Sounds"), ("Auxs", 0xa0, "Sounds"), ("Auxs", 0xa4, "Sounds"),
    ("Auxs", 0xa8, "Sounds"), ("Auxs", 0xac, "Sounds"),
    ("Sounds", 0x30, "Sounds"),
    ("Overlays", 0x0c, "Texs"), ("Overlays", 0x14, "Pics"),
]
EFFECT_REFS = [("Regions", 0xb0), ("Walls", 0xd0), ("Objects", 0x140), ("Views", 0x68)]
N_EFFECTS = 273  # entries registered in the effect table (re/engine/effects.md)

TOTAL = defaultdict(lambda: [0, 0])  # check -> [pass, fail]


class Level:
    def __init__(self, path):
        self.x = x = Xrb(path)
        self.name = os.path.basename(path)
        self.R = {s: x.recs(s) for s in x.desc}
        self.P = [(fx(p, 0x14), fx(p, 0x18), fx(p, 0x1c)) for p in self.R["Points"]]
        used = x.h32(0x20)
        self.wl = struct.unpack_from(f"<{used}i", x.body, x.off["Trailer"])
        self.tex = []
        off = x.off["TexData"]
        for t in self.R["Texs"]:
            n, fr, rot, sh, sz = u16(t, 0x1a), u16(t, 0x10), u16(t, 0x12), u16(t, 0x14), u32(t, 0x50)
            d = x.body[off:off + sz]
            off += sz
            m = fr << sh
            ents = [struct.unpack_from("<HHi", d, 12 * k) for k in range(n)]
            pics = list(struct.unpack_from(f"<{m}i", d, 12 * n))
            mirror = list(d[12 * n + 4 * m:12 * n + 5 * m])
            self.tex.append((n, fr, rot, sh, sz, ents, pics, mirror))


def check(lv, label, fails, total, hard=True, note=""):
    ok = fails == 0
    TOTAL[label][0 if ok else 1] += 1
    flag = "ok  " if ok else ("FAIL" if hard else "warn")
    print(f"  [{flag}] {label}: {total - fails}/{total} {note}")
    return ok or not hard


def region_of_point(lv, px, py, pz):
    """Engine point-in-region: nearest wall crossing on a ray towards +X (see XRB-records.md)."""
    best, res = 1e18, None
    for w in lv.R["Walls"]:
        f, b = i32(w, 0x80), i32(w, 0x84)
        if f == b:
            continue
        (x1, y1, _), (x2, y2, _) = lv.P[i32(w, 0xc)], lv.P[i32(w, 0x10)]
        if x1 < px and x2 < px:
            continue
        if y1 >= py > y2:      # wall runs towards -Y: the front region is on the ray's origin side
            side = f
        elif y1 < py <= y2:    # wall runs towards +Y: the back region is on the origin side
            side = b
        else:
            continue
        if side < 0:
            continue
        r = lv.R["Regions"][side]
        if not (fx(r, 0xc) <= pz < fx(r, 0x10)) and not (fx(r, 0xc) == fx(r, 0x10) == pz):
            continue
        xi = x1 + (x2 - x1) * (py - y1) / (y2 - y1)
        d = xi - px
        if 0 < d < best:
            best, res = d, side
    return res


def validate(path):
    lv = Level(path)
    x, R = lv.x, lv.R
    cnt = {s: x.count(s) for s in x.desc}
    print(f"{lv.name}: walls={cnt['Walls']} regions={cnt['Regions']} points={cnt['Points']} "
          f"objects={cnt['Objects']} texs={cnt['Texs']} pics={cnt['Pics']}")
    good = True

    good &= check(lv, "layout sums to file size", int(x.end != len(x.body)), 1)

    # self indices
    fails = tot = 0
    for sec, off, fn in [("Points", 8, u16), ("Walls", 8, i32), ("Regions", 8, u16),
                         ("Objects", 8, i32), ("Sounds", 8, i32), ("Auxs", 8, i32),
                         ("Overlays", 8, i32), ("Paths", 8, i32), ("Views", 8, i32),
                         ("Pics", 0x1a, u16)]:
        for k, r in enumerate(R[sec]):
            tot += 1
            fails += fn(r, off) != k
    good &= check(lv, "record self-index fields", fails, tot)

    # all resolved index fields within range
    fails = tot = 0
    bad = Counter()
    for sec, off, tgt in REFS:
        for r in R[sec]:
            v = i32(r, off)
            tot += 1
            if not (v == -1 or 0 <= v < max(cnt.get(tgt, 0), x.desc[tgt][2])):
                fails += 1
                bad[f"{sec}+{off:#x}->{tgt}"] += 1
    for sec, off in EFFECT_REFS:
        for r in R[sec]:
            v = i32(r, off)
            tot += 1
            if not (v == -1 or 0 <= v < N_EFFECTS):
                fails += 1
                bad[f"{sec}+{off:#x}->Effect"] += 1
    good &= check(lv, "index fields in range (-1 = none)", fails, tot, note=dict(bad) if bad else "")

    # walls: geometry and length
    W, P = R["Walls"], lv.P
    fails = 0
    for w in W:
        (x1, y1, _), (x2, y2, _) = P[i32(w, 0xc)], P[i32(w, 0x10)]
        fails += abs(math.hypot(x2 - x1, y2 - y1) - fx(w, 0x8c)) > 1.0
    good &= check(lv, "wall +0x8c == |p2-p1|", fails, len(W))

    # point reference counts
    refs = Counter()
    for w in W:
        refs[i32(w, 0xc)] += 1
        refs[i32(w, 0x10)] += 1
    orefs = Counter(i32(o, 0x14) for o in R["Objects"])
    fails = sum(1 for k, p in enumerate(R["Points"])
                if refs[k] and u16(p, 0xa) != refs[k])
    good &= check(lv, "point u16@0xa == #wall endpoint refs", fails,
                  sum(1 for k in range(cnt["Points"]) if refs[k]))

    # region wall lists
    Rg = R["Regions"]
    fails = 0
    listed = defaultdict(set)
    for ri, r in enumerate(Rg):
        s, n = i32(r, 0x110), i32(r, 0xbc)
        if s < 0 or s + n > len(lv.wl):
            fails += 1
            continue
        for wi in lv.wl[s:s + n]:
            listed[wi].add(ri)
            w = W[wi]
            fails += ri not in (i32(w, 0x80), i32(w, 0x84), i32(w, 0x88))
    good &= check(lv, "region wall list -> wall.front/back == region", fails, len(lv.wl))
    fails = 0
    for wi, w in enumerate(W):
        f, b = i32(w, 0x80), i32(w, 0x84)
        fails += f not in listed[wi] or (b >= 0 and b not in listed[wi])
    good &= check(lv, "every wall listed in its front (and back) region", fails, len(W))
    starts = sorted((i32(r, 0x110), i32(r, 0xbc)) for r in Rg)
    fails = sum(1 for (s0, n0), (s1, _) in zip(starts, starts[1:]) if s0 + n0 != s1)
    good &= check(lv, "region wall lists contiguous in trailer", fails, max(len(starts) - 1, 0))

    # orientation: front region lies to the right of p1->p2 (clockwise loops, X right / Y up)
    neg = pos = 0
    for ri, r in enumerate(Rg):
        s, n = i32(r, 0x110), i32(r, 0xbc)
        area = 0.0
        for wi in lv.wl[s:s + n]:
            w = W[wi]
            (ax, ay, _), (bx, by, _) = P[i32(w, 0xc)], P[i32(w, 0x10)]
            if i32(w, 0x80) != ri:
                ax, ay, bx, by = bx, by, ax, ay
            area += ax * by - bx * ay
        neg += area < 0
        pos += area > 0
    check(lv, "front side = right of p1->p2 (region signed area < 0)", pos, pos + neg, hard=False,
          note="(positive-area regions are multi-loop/odd shapes)")

    # region bounds and center
    fails = 0
    for ri, r in enumerate(Rg):
        s, n = i32(r, 0x110), i32(r, 0xbc)
        if n == 0:
            continue
        xs, ys = [], []
        for wi in lv.wl[s:s + n]:
            for o in (0xc, 0x10):
                px, py, _ = P[i32(W[wi], o)]
                xs.append(px)
                ys.append(py)
        exp = (min(xs), max(xs), min(ys), max(ys), (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2)
        got = tuple(fx(r, o) for o in (0xc4, 0xc8, 0xcc, 0xd0, 0xd4, 0xd8))
        fails += any(abs(a - b) > 1.0 for a, b in zip(exp, got))
    good &= check(lv, "region bbox minX,maxX,minY,maxY,cx,cy @0xc4..0xd8", fails,
                  sum(1 for r in Rg if i32(r, 0xbc)))

    # heights
    fails = sum(1 for r in Rg if fx(r, 0xc) > fx(r, 0x10))
    good &= check(lv, "floor(0x0c) <= ceil(0x10)", fails, len(Rg))
    fails = sum(1 for r in Rg if fx(r, 0x14) > fx(r, 0x18))
    good &= check(lv, "rest floor(0x14) <= rest ceil(0x18)", fails, len(Rg))
    doors = [r for r in Rg if (i32(r, 0xc), i32(r, 0x10)) != (i32(r, 0x14), i32(r, 0x18))]
    fails = sum(1 for r in doors if i32(r, 0xb0) != 219 or fx(r, 0x10) != fx(r, 0xc))
    check(lv, "regions with cur != rest heights are closed RTDOORs", fails, len(doors), hard=False)

    # portals
    fails = sum(1 for w in W if i32(w, 0x80) < 0)
    two = sum(1 for w in W if i32(w, 0x84) >= 0)
    selfref = sum(1 for w in W if i32(w, 0x84) == i32(w, 0x80))
    good &= check(lv, "every wall has a front region", fails, len(W),
                  note=f"(two-sided: {two}, of which back==front internal walls: {selfref})")
    fails = sum(1 for w in W if i32(w, 0x84) >= 0 and i32(w, 0x84) != i32(w, 0x80)
                and abs(fx(Rg[i32(w, 0x84)], 0xc) - fx(Rg[i32(w, 0x80)], 0xc)) > 4096)
    check(lv, "portal floor delta < 4096 (sanity)", fails, two, hard=False)
    fails = sum(1 for w in W if (i32(w, 0x84) >= 0) != bool(u32(w, 0) & 0x200))
    check(lv, "wall flag 0x200 <=> two-sided", fails, len(W), hard=False)
    fails = sum(1 for w in W if i32(w, 0x84) < 0 and i32(w, 0x44) < 0)
    check(lv, "one-sided walls have a middle texture", fails, sum(1 for w in W if i32(w, 0x84) < 0),
          hard=False)

    # stacked regions (rAbove/rBelow) symmetric
    fails = tot = 0
    for ri, r in enumerate(Rg):
        a, b = i32(r, 0x60), i32(r, 0x5c)
        if b >= 0:
            tot += 1
            fails += i32(Rg[b], 0x60) != ri
        if a >= 0:
            tot += 1
            fails += i32(Rg[a], 0x5c) != ri
    check(lv, "region 0x5c/0x60 links are mutual", fails, tot, hard=False)

    # texture slots: pic belongs to the tex's pic table
    fails = tot = 0
    for sec, slots in (("Walls", (0x2c, 0x44, 0x5c)), ("Regions", (0x1c, 0x34)),
                       ("Objects", (0x64,)), ("Overlays", (0x0c,))):
        for r in R[sec]:
            for o in slots:
                t, p = i32(r, o), i32(r, o + 8)
                if t < 0:
                    fails += p != -1 and sec != "Objects"
                    tot += 1
                    continue
                tot += 1
                fails += p not in lv.tex[t][6]
    good &= check(lv, "texture slot {tex,+4 rt,+8 pic}: pic in tex pic table", fails, tot)

    # tex data
    fails = 0
    for (n, fr, rot, sh, sz, ents, pics, mirror) in lv.tex:
        fails += sz != 12 * n + 5 * (fr << sh)
        fails += rot != 1 << sh
        fails += any(not (-1 <= p < cnt["Pics"]) for p in pics)
    for k, t in enumerate(R["Texs"]):
        fails += u16(t, 0x16) != 0x800 >> u16(t, 0x14) or u16(t, 0x18) != 11 - u16(t, 0x14)
        fails += u32(t, 0x5c) != k
        single = lv.tex[k][0] == 1 and lv.tex[k][1] == 1 and lv.tex[k][2] == 1
        fails += u32(t, 0) & 0xff not in (2, 3) or (u32(t, 0) & 0xff == 2 and not single)
    good &= check(lv, "tex header/data size consistency", fails, len(lv.tex))
    fails = sum(1 for (n, fr, rot, sh, sz, ents, pics, mirror) in lv.tex
                if any(not (nx < n or nx in (98, 99)) or f >= fr << sh for f, nx, _ in ents))
    check(lv, "tex sequence entries: frame < frames*rot, next < n or 98/99", fails, len(lv.tex),
          hard=False, note="(failures = unused/broken authoring sequences)")

    # objects
    O = R["Objects"]
    fails = sum(1 for o in O if i32(o, 0x4c) != max(i32(o, 0xfc), 0))
    check(lv, "object +0x4c == max(+0xfc, 0) (saved region index)", fails, len(O), hard=False)
    fails = sum(1 for o in O if abs(fx(o, 0x2c) - P[i32(o, 0x14)][2]) > 0.01)
    check(lv, "object z(0x2c) == point z", fails, len(O), hard=False)
    fails = sum(1 for o in O if i32(o, 0xfc) >= 0 and fx(o, 0x2c) < fx(Rg[i32(o, 0xfc)], 0xc))
    check(lv, "object z >= its region floor", fails, sum(1 for o in O if i32(o, 0xfc) >= 0), hard=False)
    # region object lists (0xb4 head, 0x14c next)
    fails = 0
    seen = set()
    for ri, r in enumerate(Rg):
        o, guard = i32(r, 0xb4), 0
        while o >= 0 and guard < 10000:
            seen.add(o)
            fails += i32(O[o], 0xfc) != ri
            o, guard = i32(O[o], 0x14c), guard + 1
    fails += sum(1 for k, o in enumerate(O) if i32(o, 0xfc) >= 0 and k not in seen)
    check(lv, "region object list (0xb4 -> obj 0x14c) == objects with 0xfc", fails, len(O), hard=False)
    # engine point-in-region
    inreg = [(k, o) for k, o in enumerate(O) if i32(o, 0xfc) >= 0 and k in seen]
    fails = 0
    for k, o in inreg:
        px, py, _ = P[i32(o, 0x14)]
        fails += region_of_point(lv, px, py, fx(o, 0x2c)) != i32(o, 0xfc)
    check(lv, "ray-cast point-in-region reproduces object region", fails, len(inreg), hard=False)

    # player / start
    p0 = O[0]
    s = O[x.h32(0x4c)]
    f = int(not (p0[2] & 4) or i32(p0, 0x140) != 0 or P[i32(p0, 0x14)] != P[i32(s, 0x14)]
            or u16(p0, 0x24) != u16(s, 0x24))
    good &= check(lv, "Objects[0]=player (flag 0x40000, effect 0 OPLAY), hdr+0x4c = start copy", f, 1)
    fails = sum(1 for o in O if not (0 <= u16(o, 0x24) < 2048))
    good &= check(lv, "object angle u16@0x24 in [0,2048)", fails, len(O))

    # pics in XPG
    xpg = os.path.getsize(path[:-4] + ".XPG")
    fails = sum(1 for p in R["Pics"] if i32(p, 0x24) != -1 and i32(p, 0x24) + u16(p, 0xe) * u16(p, 0x10) > xpg)
    good &= check(lv, "pic xpg_offset + h*w <= XPG size", fails, cnt["Pics"])
    return good


def main():
    files = sys.argv[1:] or sorted(glob.glob(os.path.join(ROOT, "original", "*.XRB")))
    ok = all([validate(p) for p in files])
    print("\nSUMMARY (files passing / failing per check)")
    for k, (a, b) in TOTAL.items():
        print(f"  {a:3d} / {b:3d}  {k}")
    print("ALL HARD CHECKS PASSED" if ok else "SOME HARD CHECKS FAILED")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
