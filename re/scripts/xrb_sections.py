"""Locate every XRB section's record array (offsets) and dump View records.

Usage: python3 -I re/scripts/xrb_sections.py original/LIPS00.XRB
Section order / strides follow ReadCompiledOrb (FUN_004d5968): Texs(+var data), Pics, Points,
Paths, Regions, Walls, Objects, ObjectStates, 0x180 bytes of state-name tables, 0x800 bytes
(512 u32), Views, ...
"""
import struct, sys

NAMES = ["Texs", "Pics", "Points", "Paths", "Regions", "Walls", "Objects", "ObjectStates",
         "Views", "Auxs", "Sounds", "Texts", "Windows", "Overlays", "ActionNodes"]

def sections(d):
    body = d[0x68:]
    descs = [struct.unpack_from("<5I", body, i * 20) for i in range(15)]
    off = 300
    out = {}
    ntex, used = descs[0][2], descs[0][1]
    out["Texs"] = (off, ntex, 100)
    texs = [body[off + i * 100: off + (i + 1) * 100] for i in range(ntex)]
    off += ntex * 100
    off += sum(struct.unpack_from("<I", t, 0x50)[0] for t in texs[:used])
    for name, idx, size in [("Pics", 1, 40), ("Points", 2, 100), ("Paths", 3, 452),
                            ("Regions", 4, 308), ("Walls", 5, 240), ("Objects", 6, 480),
                            ("ObjectStates", 7, 472)]:
        n = descs[idx][2]
        out[name] = (off, n, size)
        off += n * size
    off += 0x180 + 0x800
    out["Views"] = (off, descs[8][2], 176)
    return body, descs, out

def main(path):
    d = open(path, "rb").read()
    body, descs, out = sections(d)
    for k, (o, n, s) in out.items():
        print(f"{k:12s} body+{o:#x} n={n} size={s}")
    o, n, s = out["Views"]
    for i in range(min(n, descs[8][1])):
        r = body[o + i * s: o + (i + 1) * s]
        name = r[4:12].split(b"\0")[0].decode("latin1")
        shorts = struct.unpack_from("<88h", r)
        print(f"view {i} type={struct.unpack_from('<I', r)[0]:#x} name={name!r}")
        for j in range(0, 88, 8):
            print(f"   +{j*2:#04x}: " + " ".join(f"{v:6d}" for v in shorts[j:j + 8]))

if __name__ == "__main__":
    main(sys.argv[1])
