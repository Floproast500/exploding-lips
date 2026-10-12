"""Dump raw records of an XRB section as words, for field discovery.
usage: xrb_dump.py FILE SECTION [first] [count]   (offsets follow the loader order, see xrb_lib)"""
import struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from xrb_lib import Xrb


def main(path, section, first=0, count=4):
    x = Xrb(path)
    rl = x.desc[section][3]
    for r in range(int(first), min(int(first) + int(count), x.count(section))):
        rec = x.rec(section, r)
        print(f"--- {section}[{r}] name={rec[4:12].split(b'\0')[0]!r}")
        words = struct.unpack_from(f"<{rl // 4}i", rec)
        line = []
        for k, w in enumerate(words):
            line.append(f"{k * 4:03x}:{w:#x}" + (f"({w / 65536:.2f})" if abs(w) >= 65536 and w & 0xffff == 0 else ""))
        for k in range(0, len(line), 6):
            print("   " + "  ".join(line[k:k + 6]))


if __name__ == "__main__":
    main(*sys.argv[1:])
