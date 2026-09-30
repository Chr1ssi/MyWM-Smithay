"""Screen capture (wlr-screencopy) with grim against the nested compositor.

Needs grim and ImageMagick. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/screencopy_smoke.py
"""
import subprocess, time
from smoke_support import Compositor


def size(path):
    return subprocess.check_output(["identify", "-format", "%wx%h", path]).decode()


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()


comp = Compositor(windows=2)
try:
    time.sleep(3)
    env = comp.env()
    full, part, cursor = (f"{comp.dir}/{name}.png" for name in ("full", "part", "cursor"))
    subprocess.run(["grim", full], env=env, check=True, timeout=15)
    assert size(full) == "1280x800", size(full)
    # The gap around the tiles shows the palette background; the middle of a window does not.
    assert pixel(full, 2, 2) == "#1E1E2E", pixel(full, 2, 2)
    assert pixel(full, 320, 400) != "#1E1E2E"
    # A region.
    subprocess.run(["grim", "-g", "100,100 200x150", part], env=env, check=True, timeout=15)
    assert size(part) == "200x150", size(part)
    # With the cursor overlaid (a different request flag).
    subprocess.run(["grim", "-c", cursor], env=env, check=True, timeout=15)
    assert size(cursor) == "1280x800"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
