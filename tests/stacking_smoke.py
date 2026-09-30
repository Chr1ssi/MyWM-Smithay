"""Stacking order: fullscreen windows above the bar (Top layer), borders below overlays.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/stacking_smoke.py
"""
import subprocess, time
from smoke_support import Compositor, wait

CLIENT = "./target/debug/mywm-test-client"


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()


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
print("OK")
