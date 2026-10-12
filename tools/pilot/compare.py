#!/usr/bin/env python3
"""Compare screenshots of the original game and the reimplementation taken by pilot.py.

    compare.py re/shots/original re/shots/ours --out re/shots/compare [--threshold 0.12] [--crop WxH+X+Y]

For every <name>.png present in both directories it writes <out>/<name>.png (original | ours |
difference) and prints the normalised RMSE (0 = identical, 1 = opposite). Exits non-zero when
any pair exceeds --threshold, so it can gate CI.
"""
import argparse
import subprocess
import sys
from pathlib import Path


def rmse(a: Path, b: Path, crop: str | None) -> float:
    args = ["magick", str(a), str(b)]
    if crop:
        args += ["-crop", crop, "+repage"]
    args += ["-metric", "RMSE", "-compare", "-format", "%[distortion]", "info:"]
    out = subprocess.run(args, capture_output=True, text=True)
    text = (out.stdout or out.stderr).strip()
    # ImageMagick prints either "<abs> (<normalised>)" or just the normalised value.
    if "(" in text:
        text = text.split("(")[1].rstrip(")")
    return float(text)


def side_by_side(a: Path, b: Path, out: Path):
    subprocess.run(["magick", str(a), str(b), "(", str(a), str(b), "-compose", "difference", "-composite",
                    "-auto-level", ")", "+append", str(out)], check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("original", type=Path)
    ap.add_argument("ours", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--threshold", type=float, default=0.12)
    ap.add_argument("--crop", help="ImageMagick geometry to compare only part of the frame, e.g. 472x328+80+60")
    a = ap.parse_args()

    a.out.mkdir(parents=True, exist_ok=True)
    names = sorted(p.name for p in a.original.glob("*.png") if (a.ours / p.name).exists())
    if not names:
        sys.exit("no screenshot pairs found")
    failed = 0
    for name in names:
        score = rmse(a.original / name, a.ours / name, a.crop)
        side_by_side(a.original / name, a.ours / name, a.out / name)
        status = "ok" if score <= a.threshold else "DIFF"
        failed += status != "ok"
        print(f"{status:4} {score:.4f}  {name}")
    print(f"{len(names) - failed}/{len(names)} within threshold {a.threshold}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
