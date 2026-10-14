"""Edge markers: a strip in the active border's color at a monitor edge that tiled windows reach beyond.

Three windows on one monitor leave one column beyond an edge; the bar protocol says which edge, and the
compositor draws the strip there (the shell no longer opens a panel window for it).
Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/edge_markers_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor

ACCENT = "#89B4FA"  # the default active border


def pixel(comp, x, y):
    shot = subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(),
                          capture_output=True, check=True, timeout=15).stdout
    r, g, b = shot[-3:]
    return f"#{r:02X}{g:02X}{b:02X}"


comp = Compositor(windows=3)
try:
    bar = Bar(comp.sock)
    time.sleep(4)
    # The first column focused: the row scrolls back and the last window reaches beyond the right edge.
    comp.key("alt+h")
    comp.key("alt+h")
    bar.until(lambda s: s[0]["right"] and not s[0]["left"])
    time.sleep(0.5)
    assert pixel(comp, 1278, 400) == ACCENT, pixel(comp, 1278, 400)
    assert pixel(comp, 1, 400) != ACCENT, pixel(comp, 1, 400)
    # The last column focused: now the first one lies beyond the left edge.
    comp.key("alt+l")
    comp.key("alt+l")
    bar.until(lambda s: s[0]["left"] and not s[0]["right"])
    time.sleep(0.5)
    assert pixel(comp, 1, 400) == ACCENT, pixel(comp, 1, 400)
    assert pixel(comp, 1278, 400) != ACCENT, pixel(comp, 1278, 400)
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
