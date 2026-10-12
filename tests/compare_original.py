#!/usr/bin/env python3
"""Golden comparison against the original game.

Pilots lips.exe under Wine to the LIPS00 start (tests/pilot/start_lips00.txt), checks the
player's memory-probed position matches our level start, renders our engine from the same
spot with `lipstool render`, and compares the 3D view (HUD/weapon areas masked) at 320x200.

    compare_original.py [--threshold 0.08] [--reuse]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHOTS = ROOT / "re" / "shots" / "golden"
# 3D area of the original 320x200 screen that has no HUD text or weapon: left/top part.
COMPARE_CROP = "250x165+0+0"


def run(*cmd, **kw):
    return subprocess.run(cmd, check=True, cwd=ROOT, **kw)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--threshold", type=float, default=0.08)
    ap.add_argument("--reuse", action="store_true", help="reuse the last original screenshot")
    a = ap.parse_args()
    SHOTS.mkdir(parents=True, exist_ok=True)

    original = SHOTS / "02_lips00.png"
    if not (a.reuse and original.exists()):
        run(sys.executable, "-I", "tools/pilot/pilot.py", "--exe", "original", "--script", "tests/pilot/start_lips00.txt", "--out", str(SHOTS))
    probe = json.loads((SHOTS / "start.json").read_text()) if (SHOTS / "start.json").exists() else None
    if probe:
        print(f"original player: {probe}")
        assert (probe["x"], probe["y"], probe["region"], probe["angle"]) == (11008.0, 15616.0, 640, 512), "start position differs"

    walked = json.loads((SHOTS / "walked.json").read_text()) if (SHOTS / "walked.json").exists() else None
    turned = json.loads((SHOTS / "turned.json").read_text()) if (SHOTS / "turned.json").exists() else None
    if probe and walked and turned:
        # Key-hold timing through X is approximate, so compare rates loosely: walking is
        # 800 units / 1.2 s (667/s), turning 24 angle units per 30 fps frame (720/s).
        dist = ((walked["x"] - probe["x"]) ** 2 + (walked["y"] - probe["y"]) ** 2) ** 0.5
        turn = (turned["angle"] - walked["angle"]) % 2048
        print(f"original: walked {dist:.0f} units in ~0.5 s, turned {turn} units in ~0.4 s")
        assert 0.6 * 333 <= dist <= 1.4 * 333, f"walk distance {dist}"
        assert 0.6 * 288 <= turn <= 1.4 * 288, f"turn {turn}"

    ours = SHOTS / "ours_lips00.png"
    run("target/release/lipstool", "render", "original", "LIPS00", str(ours), "320x200")
    orig320 = SHOTS / "orig_lips00_320.png"
    # Under Wine the 320x200 screen is stretched to 472x328 at (80,60) of the 640x480 window.
    run("magick", str(original), "-crop", "472x328+80+60", "+repage", "-resize", "320x200!", str(orig320))
    def rmse(*extra):
        result = subprocess.run(
            ["magick", str(orig320), str(ours), *extra, "-metric", "RMSE", "-compare", "-format", "%[distortion]", "info:"],
            cwd=ROOT, capture_output=True, text=True,
        )
        text = (result.stdout or result.stderr).strip()
        return float(text.split("(")[1].rstrip(")")) if "(" in text else float(text)

    score = rmse("-crop", COMPARE_CROP, "+repage")
    # Whole screen (3D view, HUD, weapon) with the clock digits blanked in both images.
    full = rmse("-fill", "black", "-draw", "rectangle 276,46 320,58")
    print(f"LIPS00 start, full screen incl. HUD and weapon: RMSE {full:.4f}")
    run("magick", str(orig320), str(ours), "+append", "-filter", "point", "-resize", "200%", str(SHOTS / "side_by_side.png"))
    print(f"LIPS00 start view RMSE {score:.4f} (threshold {a.threshold})")
    sys.exit(0 if score <= a.threshold and full <= a.threshold else 1)


if __name__ == "__main__":
    main()
