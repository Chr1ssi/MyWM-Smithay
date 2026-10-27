"""Stacking order: fullscreen windows above the bar (Top layer), borders below overlays and
below any window in front of their own (a dialog).

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/stacking_smoke.py
"""
import subprocess, time
from smoke_support import Compositor, wait

CLIENT = "./target/debug/splice-test-client"


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()


def pixel_row(path, y, x0, x1):
    """The colors of pixels x0..x1 (exclusive) of row y."""
    out = subprocess.check_output(["convert", path, "-crop", "%dx1+%d+%d" % (x1 - x0, x0, y), "+repage", "txt:-"]).decode()
    return [line.split()[2].upper() for line in out.strip().splitlines()[1:]]


def shot(comp, name):
    path = f"{comp.dir}/{name}.png"
    subprocess.run(["grim", path], env=comp.env(), check=True, timeout=15)
    return path


def count_windows(comp):
    return open(comp.dir + "/log").read().count("new window")


# --- A fullscreen window covers the bar; without it the bar shows.
comp = Compositor(extra_args=f"{CLIENT} layer top 400 40 FF0000 60 & sleep 1; ")
try:
    time.sleep(3)
    assert pixel(shot(comp, "bar"), 640, 400) == "#FF0000", "the bar is not drawn"
    subprocess.Popen([CLIENT, "fullscreen", "00FF00", "30"], env=comp.env(), stdout=subprocess.DEVNULL)
    assert wait(lambda: count_windows(comp) >= 1, 10)
    time.sleep(2)
    assert pixel(shot(comp, "fullscreen"), 640, 400) == "#00FF00", "the bar is drawn over the fullscreen window"
    assert comp.stop()
    print("fullscreen above the bar OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# --- Window borders do not show through an overlay (the launcher).
comp = Compositor(windows=2)
try:
    time.sleep(3)
    before = shot(comp, "before")
    border = {"#89B4FA", "#45475A"}
    row = [pixel(before, x, 400) for x in range(430, 850, 2)]
    assert any(p in border for p in row), "no window border in the middle of the screen"
    subprocess.Popen([CLIENT, "layer", "overlay", "420", "300", "0000FF", "30"], env=comp.env(), stdout=subprocess.DEVNULL)
    time.sleep(2.5)
    after = shot(comp, "after")
    assert pixel(after, 640, 400) == "#0000FF", "the overlay is not drawn"
    row = [pixel(after, x, 400) for x in range(440, 840, 2)]
    assert not any(p in border for p in row), "a window border is drawn over the overlay"
    assert comp.stop()
    print("borders below the overlay OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# --- A floating window (a dialog) covers the borders of the tiled windows behind it.
comp = Compositor(windows=2, config="[[rules]]\napp_id = 'splice.test.float'\nfloating = true\n")
try:
    time.sleep(3)
    before = shot(comp, "tiled")
    border = {"#89B4FA", "#45475A"}
    columns = [300 + i for i, p in enumerate(pixel_row(before, 400, 300, 980)) if p in border]
    assert columns, "no window border in the middle of the screen"
    # Its buffer is black, as nothing else on the screen is.
    subprocess.Popen([CLIENT, "named", "30", "splice.test.float", "-"], env=comp.env(), stdout=subprocess.DEVNULL)
    assert wait(lambda: count_windows(comp) >= 3, 10)
    time.sleep(2)
    after = shot(comp, "dialog")
    box = subprocess.check_output(["convert", after, "-fill", "white", "+opaque", "black", "-format", "%@", "info:"]).decode()
    w, rest = box.split("x")
    h, x, y = rest.split("+")
    x, y, w, h = int(x), int(y), int(w), int(h)
    covered = [c for c in columns if x < c < x + w - 1]
    assert covered and y < 400 < y + h, f"the floating window {(x, y, w, h)} does not cover a border at {columns}"
    shown = [pixel(after, c, 400) for c in covered]
    assert all(p == "#000000" for p in shown), f"a tiled window's border is drawn over the floating window: {shown}"
    assert comp.stop()
    print("borders behind a floating window OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
