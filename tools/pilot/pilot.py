#!/usr/bin/env python3
"""Drive a game on a headless X display (Xvfb) with scripted input and screenshots.

Used to pilot the original lips.exe under Wine and our reimplementation with the same script,
so their screenshots can be compared.

    pilot.py --exe original  --script tests/pilot/boot.txt --out re/shots/original
    pilot.py --exe ours      --script tests/pilot/boot.txt --out re/shots/ours

Script commands (one per line, '#' comments):
    wait <seconds>             sleep
    key <xdotool keysym>...    tap keys, e.g. `key Return`, `key Down Down Return`
    hold <keysym> <seconds>    hold a key down
    type <text>                type text
    click <x> <y> [button]     click at window coordinates
    move <x> <y>               move the mouse to window coordinates (menus select under the hand)
    shot <name>                save <out>/<name>.png
    probe <name>               (original only) read the player's position/angle/region from
                               the game's memory into <out>/<name>.json
    poke <x> <y> <z> <angle> <region> [obj]
                               (original only) teleport the player; later probes also report
                               object index `obj` (mover hit counter, effect)
"""
import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DISPLAY = ":99"
WIDTH, HEIGHT = 640, 480


def headless_env():
    """Environment that pins every client to the Xvfb display (never the user's desktop)."""
    env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET")}
    return env | {"DISPLAY": DISPLAY}


def run(cmd, **kw):
    return subprocess.run(cmd, env=headless_env(), **kw)


def start_display():
    if run(["xdotool", "getdisplaygeometry"], capture_output=True).returncode == 0:
        return None  # display already running
    proc = subprocess.Popen(["Xvfb", DISPLAY, "-screen", "0", f"{WIDTH}x{HEIGHT}x24", "-nolisten", "tcp"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
    return proc


def launch(exe, extra_args=()):
    env = headless_env()
    if exe == "original":
        run_dir = ROOT / "re" / "run"
        if not run_dir.exists():
            shutil.copytree(ROOT / "original", run_dir)
        env |= {"WINEPREFIX": str(ROOT / "re" / "wineprefix"), "WINEDEBUG": os.environ.get("WINEDEBUG", "-all")}
        # Fresh save/preferences state for reproducible runs.
        for name in ("LIPSTART.GAM", "GODDRUN.DAT", "goddrun.tad"):
            if (ROOT / "original" / name).exists():
                shutil.copy(ROOT / "original" / name, run_dir / name)
        prepare_bios_font(run_dir)
        log = open(ROOT / "re" / "wine-run.log", "w")
        return subprocess.Popen(["wine", "godd-launch.exe", "lips.exe"], cwd=run_dir, env=env,
                                stdout=log, stderr=log, start_new_session=True)
    # Keep settings/saves of test runs out of the user's real config directory.
    config = ROOT / "target" / "pilot-config"
    shutil.rmtree(config, ignore_errors=True)
    env["XDG_CONFIG_HOME"] = str(config)
    binary = ROOT / "target" / "release" / "biglips"
    return subprocess.Popen([str(binary), "--data", str(ROOT / "original"), "--size", f"{WIDTH}x{HEIGHT}", "--mute", *extra_args],
                            cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)


def prepare_bios_font(run_dir):
    """Put godd-launch.exe and biosfont.bin next to lips.exe (see tools/pilot/launcher)."""
    launcher_src = ROOT / "tools" / "pilot" / "launcher"
    exe = launcher_src / "godd-launch.exe"
    source = launcher_src / "godd-launch.c"
    if not exe.exists() or exe.stat().st_mtime < source.stat().st_mtime:
        subprocess.run(["i686-w64-mingw32-gcc", "-O2", "-o", str(exe), str(launcher_src / "godd-launch.c")], check=True)
    shutil.copy(exe, run_dir / "godd-launch.exe")
    font = run_dir / "biosfont.bin"
    if not font.exists():
        subprocess.run(["cargo", "run", "-q", "-p", "lipstool", "--", "biosfont", str(font)], cwd=ROOT, check=True)


def game_window(timeout=20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        out = run(["xdotool", "search", "--onlyvisible", "--name", "."], capture_output=True, text=True).stdout.split()
        best = None
        for wid in out:
            geo = run(["xdotool", "getwindowgeometry", wid], capture_output=True, text=True).stdout
            if "Geometry:" in geo:
                w, h = map(int, geo.split("Geometry:")[1].split()[0].split("x"))
                if w * h >= 320 * 200 and (best is None or w * h > best[1]):
                    best = (wid, w * h)
        if best:
            return best[0]
        time.sleep(0.25)
    raise SystemExit("game window did not appear")


# Addresses in lips.exe (re/formats/XRB-records.md): pointer arrays filled by the loader.
OBJECTS_PTR_ARRAY = 0x0056774C


def probe_original():
    """Ask godd-launch.exe (the game's Windows parent) to read the player object from memory."""
    run_dir = ROOT / "re" / "run"
    answer = run_dir / "probe.json"
    answer.unlink(missing_ok=True)
    (run_dir / "probe.req").write_text("")
    deadline = time.time() + 3
    while time.time() < deadline:
        if answer.exists():
            try:
                return json.loads(answer.read_text())
            except json.JSONDecodeError:
                pass
        time.sleep(0.05)
    print("probe: no answer from godd-launch.exe")
    return None


SEND_EVENTS = True  # set per executable in main()


def target(wid):
    if SEND_EVENTS:
        return ["--window", wid]
    run(["xdotool", "windowfocus", "--sync", wid], capture_output=True)
    return []


def execute(script, out, wid):
    out.mkdir(parents=True, exist_ok=True)
    for lineno, raw in enumerate(script.read_text().splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        cmd, *args = line.split()
        if cmd == "wait":
            time.sleep(float(args[0]))
        # Wine accepts synthetic events sent to its window (xdotool --window); winit ignores
        # those, so our engine gets real XTEST input on the focused window instead.
        elif cmd == "key":
            for k in args:
                run(["xdotool", "key", *target(wid), "--delay", "80", k])
                time.sleep(0.15)
        elif cmd == "hold":
            run(["xdotool", "keydown", *target(wid), args[0]])
            time.sleep(float(args[1]))
            run(["xdotool", "keyup", *target(wid), args[0]])
        elif cmd == "type":
            run(["xdotool", "type", *target(wid), "--delay", "80", line.split(None, 1)[1]])
        elif cmd == "click":
            button = args[2] if len(args) > 2 else "1"
            run(["xdotool", "mousemove", "--window", wid, args[0], args[1], "click", button])
        elif cmd == "move":
            run(["xdotool", "mousemove", "--window", wid, args[0], args[1]])
        elif cmd == "probe":
            state = probe_original() if SEND_EVENTS else None
            if state is not None:
                (out / f"{args[0]}.json").write_text(json.dumps(state, indent=2))
                print(f"probe {args[0]}: {state}")
        elif cmd == "poke":
            if SEND_EVENTS:
                (ROOT / "re" / "run" / "poke.req").write_text(" ".join(args))
                time.sleep(0.3)
        elif cmd == "shot":
            path = out / f"{args[0]}.png"
            run(["import", "-window", "root", str(path)], check=True)
            print(f"shot {path}")
        else:
            raise SystemExit(f"{script}:{lineno}: unknown command {cmd!r}")


def main():
    global WIDTH, HEIGHT, SEND_EVENTS
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", choices=["original", "ours"], required=True)
    ap.add_argument("--script", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--args", default="", help="extra arguments for our engine, e.g. '--level LIPS00 --classic'")
    ap.add_argument("--size", default=f"{WIDTH}x{HEIGHT}", help="virtual display size")
    a = ap.parse_args()
    WIDTH, HEIGHT = map(int, a.size.split("x"))
    SEND_EVENTS = a.exe == "original"

    xvfb = start_display()
    game = launch(a.exe, a.args.split())
    try:
        wid = game_window()
        run(["xdotool", "windowactivate", "--sync", wid], capture_output=True)
        execute(a.script, a.out, wid)
    finally:
        os.killpg(game.pid, signal.SIGTERM)
        if a.exe == "original":
            subprocess.run(["wineserver", "-k"], env={**os.environ, "WINEPREFIX": str(ROOT / "re" / "wineprefix")})
        if xvfb:
            xvfb.terminate()


if __name__ == "__main__":
    sys.exit(main())
