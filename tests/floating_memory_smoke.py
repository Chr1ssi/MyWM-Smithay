"""An app's floating window opens where the last one was, also after a restart.

Needs `cargo build --workspace`, grim and ImageMagick. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/floating_memory_smoke.py
"""
import os, subprocess, tempfile, time
from smoke_support import Compositor, wait

CLIENT = "./target/debug/splice-test-client"
RULE = "[[rules]]\napp_id = 'splice.test.float'\nfloating = true\n"


def black_box(comp):
    """Where the test window is: its buffer is black, everything else is not."""
    shot = tempfile.mktemp(suffix=".png")
    subprocess.run(["grim", shot], env=comp.env(), check=True)
    box = subprocess.check_output(["convert", shot, "-fill", "white", "+opaque", "black", "-format", "%@", "info:"]).decode()
    w, rest = box.split("x")
    h, x, y = rest.split("+")
    return int(x), int(y), int(w), int(h)


def open_window(comp):
    subprocess.Popen([CLIENT, "named", "60", "splice.test.float", "-"], env=comp.env(), stdout=subprocess.DEVNULL)
    assert wait(lambda: "new window" in open(comp.dir + "/log").read(), 10)
    time.sleep(1.5)


def xdotool(*args):
    subprocess.run(["xdotool", *args], check=True)
    time.sleep(0.2)


first = Compositor(config=RULE)
try:
    open_window(first)
    x, y, w, h = black_box(first)
    print("opened at", (x, y, w, h))
    win = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    # Super+drag (Alt nested) moves a floating window; the nested window needs the keyboard for Alt.
    xdotool("windowfocus", win)
    xdotool("mousemove", "--window", win, str(x + w // 2), str(y + h // 2))
    xdotool("keydown", "alt")
    xdotool("mousedown", "1")
    xdotool("mousemove", "--window", win, str(x + w // 2 + 200), str(y + h // 2 + 120))
    xdotool("mousemove", "--window", win, str(x + w // 2 + 230), str(y + h // 2 + 140))
    xdotool("mouseup", "1")
    xdotool("keyup", "alt")
    time.sleep(0.5)
    moved = black_box(first)
    print("moved to", moved)
    assert abs(moved[0] - x - 230) <= 2 and abs(moved[1] - y - 140) <= 2, moved
    remembered = open(f"{first.dir}/state/splice/floating").read()
    assert "splice.test.float" in remembered, remembered
    assert first.stop()
except BaseException:
    first.process.kill()
    print(open(first.dir + "/log").read()[-3000:])
    raise

# A new session with the same state directory: the app's window opens where it was moved to.
second = Compositor(config=RULE, extra_env={"XDG_STATE_HOME": f"{first.dir}/state"})
try:
    open_window(second)
    again = black_box(second)
    print("after a restart at", again)
    assert again == moved, (again, moved)
    assert second.stop()
except BaseException:
    second.process.kill()
    print(open(second.dir + "/log").read()[-3000:])
    raise
print("OK")
