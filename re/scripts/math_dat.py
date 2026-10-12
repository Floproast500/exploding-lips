"""Validate the MATH.DAT layout documented in re/formats/MATH.md.

Usage: python3 -I re/scripts/math_dat.py original/MATH.DAT
Layout (loaded by FUN_00480b8f):
  0x0000  2048 x i32  sin table, 16.16, index = angle & 0x7ff (2048 units per circle)
  0x2000  2048 x i32  cos table, 16.16
  0x4000  1024 x i32  "ITan" (arctangent) table: angle (16.16, 2048 units/circle) of
                      atan((i - 512) / 512), i.e. slope -1 .. +1023/512
"""
import math, struct, sys

def main(path):
    d = open(path, "rb").read()
    assert len(d) == 0x5000, len(d)
    sin = struct.unpack_from("<2048i", d, 0)
    cos = struct.unpack_from("<2048i", d, 0x2000)
    itan = struct.unpack_from("<1024i", d, 0x4000)
    K = 2 * math.pi / 2048
    def worst(table, f):
        errs = [abs(v - f(i)) for i, v in enumerate(table)]
        m = max(errs)
        return m, errs.index(m)
    for name, table, f in [
        ("sin", sin, lambda i: 65536 * math.sin(i * K)),
        ("cos", cos, lambda i: 65536 * math.cos(i * K)),
        ("itan", itan, lambda i: math.atan((i - 512) / 512) * (1024 / math.pi) * 65536),
    ]:
        m, at = worst(table, f)
        exact = sum(1 for i, v in enumerate(table) if v == int(f(i)))     # truncation toward 0
        rnd = sum(1 for i, v in enumerate(table) if v == round(f(i)))
        print(f"{name:5s} n={len(table)} max|err|={m:.3f} (at {at}) "
              f"matches trunc={exact} round={rnd}")
        assert m < 2.0, name
    print("sin[512] =", sin[512], " cos[0] =", cos[0], " itan[0] =", itan[0], " itan[512] =", itan[512])
    print("OK: layout confirmed")

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "original/MATH.DAT")
