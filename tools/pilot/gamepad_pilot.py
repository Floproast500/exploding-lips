"""Virtual Xbox 360-style gamepad (uinput) driving lips-desktop on Xvfb; screenshots each step.

Needs the uinput kernel module (`sudo modprobe uinput`) and write access to /dev/uinput.
    python3 tools/pilot/gamepad_pilot.py . re/shots/gamepad
Then compare 0_start/1_look_right/2_walk_forward/3_fire/4_menu_start.
"""
import os, subprocess, sys, time
from pathlib import Path
from evdev import UInput, AbsInfo, ecodes as e

root, out = str(Path(sys.argv[1]).resolve()), str(Path(sys.argv[2]).resolve())
os.makedirs(out, exist_ok=True)
stick = AbsInfo(0, -32768, 32767, 16, 128, 0)
trig = AbsInfo(0, 0, 255, 0, 0, 0)
caps = {
    e.EV_KEY: [e.BTN_SOUTH, e.BTN_EAST, e.BTN_NORTH, e.BTN_WEST, e.BTN_TL, e.BTN_TR,
               e.BTN_SELECT, e.BTN_START, e.BTN_MODE, e.BTN_THUMBL, e.BTN_THUMBR],
    e.EV_ABS: [(e.ABS_X, stick), (e.ABS_Y, stick), (e.ABS_RX, stick), (e.ABS_RY, stick),
               (e.ABS_Z, trig), (e.ABS_RZ, trig),
               (e.ABS_HAT0X, AbsInfo(0, -1, 1, 0, 0, 0)), (e.ABS_HAT0Y, AbsInfo(0, -1, 1, 0, 0, 0))],
}
ui = UInput(caps, name="Microsoft X-Box 360 pad", vendor=0x045e, product=0x028e, version=0x110, bustype=e.BUS_USB)
print("virtual pad", ui.device.path, flush=True)
time.sleep(1.5)
env = {k: v for k, v in os.environ.items() if k != "WAYLAND_DISPLAY"}
env.update(DISPLAY=":99", XDG_CONFIG_HOME=out + "/cfg", RUST_LOG="info")
game = subprocess.Popen([root + "/target/release/lips-desktop", "--data", root + "/original", "--software",
                         "--mute", "--level", "LIPS00", "--size", "640x480"], env=env,
                        stdout=open(out + "/game.log", "w"), stderr=subprocess.STDOUT)
def shot(n):
    subprocess.run(["import", "-window", "root", f"{out}/{n}.png"], env=env, check=True)
def axis(code, v):
    ui.write(e.EV_ABS, code, v); ui.syn()
try:
    time.sleep(8); shot("0_start")
    axis(e.ABS_RX, 32767); time.sleep(0.6); axis(e.ABS_RX, 0); time.sleep(0.5); shot("1_look_right")
    axis(e.ABS_Y, -32767); time.sleep(1.0); axis(e.ABS_Y, 0); time.sleep(0.5); shot("2_walk_forward")
    axis(e.ABS_RZ, 255); time.sleep(0.25); shot("3_fire"); axis(e.ABS_RZ, 0); time.sleep(0.5)
    ui.write(e.EV_KEY, e.BTN_START, 1); ui.syn(); time.sleep(0.1); ui.write(e.EV_KEY, e.BTN_START, 0); ui.syn()
    time.sleep(1.0); shot("4_menu_start")
finally:
    game.terminate(); game.wait(); ui.close()
