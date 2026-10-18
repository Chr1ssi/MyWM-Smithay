"""A rotated monitor (a virtual output turned by wlr-randr) in captures and in the overview.

Captures and the overview ask for an output's size while it redraws; on hardware that is when the backend is
taken out of the state, so this has to hold without asking the backend.

Needs grim, wlr-randr and ImageMagick. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/rotated_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor

SURFACE = "#313244"  # default `surface` color: the background of an (empty) cell
BACKGROUND = "#1E1E2E"  # the desktop without a wallpaper


def capture(comp, name):
    path = f"{comp.dir}/{name}.png"
    subprocess.run(["grim", "-o", "virtual-1", path], env=comp.env(), check=True, timeout=15)
    return path


def size(path):
    w, h = subprocess.check_output(["identify", "-format", "%w %h", path]).decode().split()
    return int(w), int(h)


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    # Without alpha: grim leaves the parts of its picture that no capture covered transparent.
    return out.strip().splitlines()[-1].split()[2].upper()[:7]


comp = Compositor(windows=1, extra_env={"MYWM_VIRTUAL_OUTPUTS": "1"})
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 2 and s[0]["workspaces"].get(1))
    time.sleep(1)
    # The only window, last in its row, goes to the monitor on the right.
    comp.key("alt+shift+Right")
    bar.until(lambda s: s[1]["workspaces"][2] and not s[0]["workspaces"][1])
    mode = (bar.state[1]["width"], bar.state[1]["height"])
    assert mode[0] > mode[1], f"the virtual output is not landscape: {mode}"
    done = subprocess.run(["wlr-randr", "--output", "virtual-1", "--transform", "270"], env=comp.env(), capture_output=True, text=True, timeout=15)
    assert done.returncode == 0, done.stderr
    bar.until(lambda s: (s[1]["width"], s[1]["height"]) == (mode[1], mode[0]), timeout=8)
    time.sleep(1)

    desktop = capture(comp, "desktop")
    assert size(desktop) == (mode[1], mode[0]), f"the capture is {size(desktop)}, not upright {(mode[1], mode[0])}"
    # The tiled window reaches the bottom corners: a picture cut to the mode's landscape size leaves them empty.
    inside = [pixel(desktop, x, y) for x, y in ((mode[1] // 2, mode[0] // 2), (mode[1] - 40, mode[0] - 40), (40, mode[0] - 40))]
    assert all(c != BACKGROUND for c in inside), f"the window does not fill the rotated monitor: {inside}"
    print("rotated capture upright and complete OK")

    # The overview on the rotated monitor: dimmed all the way down, cells in its upright shape (the third one, the
    # empty scratchpad, is plain `surface`).
    comp.key("alt+Tab")
    time.sleep(0.6)
    overview = capture(comp, "overview")
    bottom = pixel(overview, mode[1] // 2, mode[0] - 5)
    assert bottom != BACKGROUND, f"the overview does not dim the bottom of the rotated monitor: {bottom}"
    column = subprocess.check_output(["convert", overview, "-crop", "1x%d+%d+0" % (mode[0], mode[1] * 5 // 6), "+repage", "txt:-"]).decode()
    cell = [line.split()[2].upper() == SURFACE for line in column.strip().splitlines()[1:]]
    height, width = sum(cell), mode[1] // 4
    assert height > width, f"the scratchpad's cell is not upright: {height} rows tall"
    comp.key("Escape")
    assert comp.stop()
    print("overview on the rotated monitor OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
