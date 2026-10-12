"""Prototype XRB parser used to validate the format notes before writing the Rust version."""
import struct, sys, os

SECTIONS = ["Texs", "Pics", "Points", "Paths", "Regions", "Walls", "Objects", "ObjectStates",
            "Views", "Auxs", "Sounds", "Texts", "Windows", "Overlays", "ActionNodes"]

def probe(path):
    d = open(path, "rb").read()
    hdr = d[:0x68]
    build = struct.unpack_from("<I", hdr, 0x5c)[0]
    xpg_size = struct.unpack_from("<I", hdr, 0x34)[0]
    body = d[0x68:]
    print(f"{os.path.basename(path)}: magic={hdr[:4]!r} src={hdr[4:0x20].split(b'\0')[0].decode()} "
          f"build={build:#x} xpg_size={xpg_size} body={len(body)}")
    descs = [struct.unpack_from("<5I", body, i * 20) for i in range(15)]
    for name, desc in zip(SECTIONS, descs):
        print(f"   {name:13s} {desc}")
    off = 300
    ntex, ntex_loaded = descs[0][2], descs[0][1]
    texs = [body[off + i * 100: off + (i + 1) * 100] for i in range(ntex)]
    off += ntex * 100
    extra = 0
    for t in texs[:ntex_loaded]:
        n12 = struct.unpack_from("<H", t, 0x1a)[0]
        w = struct.unpack_from("<H", t, 0x10)[0]
        sh = struct.unpack_from("<H", t, 0x14)[0]
        size50 = struct.unpack_from("<I", t, 0x50)[0]
        calc = n12 * 12 + (w << sh) * 5
        if size50 != calc:
            print("   tex size mismatch", t[:12], n12, w, sh, size50, calc)
        extra += size50
    print(f"   tex names: {[t[:12].split(b'\\0')[0].decode('latin1') for t in texs[:6]]}")
    off += extra
    npic, npic_loaded = descs[1][2], descs[1][1]
    pics = [body[off + i * 40: off + (i + 1) * 40] for i in range(npic)]
    xpg = os.path.getsize(path[:-4] + ".XPG")
    bad = 0
    for p in pics[:npic_loaded]:
        w, h = struct.unpack_from("<HH", p, 0x0e)
        xoff = struct.unpack_from("<i", p, 0x24)[0]
        if xoff != -1 and xoff + w * h > xpg:
            bad += 1
    print(f"   pics: {npic} (loaded {npic_loaded}), out-of-range={bad}, "
          f"first: {[(p[:12].split(b'\\0')[0].decode('latin1'), struct.unpack_from('<HHi', p, 0x0e)[:2], struct.unpack_from('<i', p, 0x24)[0]) for p in pics[:3]]}")

for p in sys.argv[1:]:
    probe(p)
