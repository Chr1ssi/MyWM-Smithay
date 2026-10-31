"""The cyberdeck style's window frames: the top left and bottom right corners are cut instead of rounded, brackets
stand outside the top right and bottom left corners, and the shadow becomes a glow in the border's color. The default
style keeps its rounded corners and black shadows. Checked with grim against the nested compositor.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/frames_smoke.py
"""
import subprocess, time
from smoke_support import Compositor

BACKGROUND = (0x1E, 0x1E, 0x2E)
# One window with 24 pixels around it: its frame runs from (24, 24) to (1256, 776).
GAPS = "gaps_outer = 24\ngaps_inner = 24\n"


def pixels(comp, points):
    shot = subprocess.run(["grim", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True, timeout=15).stdout
    header, rest = shot.split(b"\n255\n", 1)
    width = int(header.split()[1])
    return [tuple(rest[(y * width + x) * 3:(y * width + x) * 3 + 3]) for x, y in points]


def look(style, effects):
    config = (f'[appearance]\nstyle = "{style}"\n' + GAPS + "[effects]\n" + effects)
    comp = Compositor(config=config, windows=1)
    try:
        time.sleep(3)
        names = ["top left", "bottom right", "top right", "bottom left", "bracket", "past the bracket", "side", "far"]
        points = [(26, 26), (1253, 773), (1253, 26), (26, 773), (1259, 30), (1259, 100), (18, 400), (4, 400)]
        return comp, dict(zip(names, pixels(comp, points)))
    except BaseException:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
        raise


# Cyberdeck, no shadow: cut corners show the background, the others the window; a bracket stands where the
# round style has nothing.
comp, deck = look("cyberdeck", "corner_radius = 20\n")
assert comp.stop()
print(deck)
assert deck["top left"] == BACKGROUND and deck["bottom right"] == BACKGROUND, deck
assert deck["top right"] != BACKGROUND and deck["bottom left"] != BACKGROUND, deck
assert deck["bracket"] != BACKGROUND, deck
assert deck["past the bracket"] == BACKGROUND, deck

comp, round_ = look("default", "corner_radius = 20\n")
assert comp.stop()
assert round_["bracket"] == BACKGROUND, round_
# A rounded corner of 20 pixels shows the background in the corner of the frame, and at the other corners too.
assert all(round_[corner] == BACKGROUND for corner in ("top left", "bottom right", "top right", "bottom left")), round_

# The glow takes the border's color (blue for the focused window); the plain shadow only darkens.
comp, glow = look("cyberdeck", "corner_radius = 20\nshadow = 24\n")
assert comp.stop()
print(glow)
assert glow["side"][2] - glow["side"][0] > BACKGROUND[2] - BACKGROUND[0] + 6, glow
comp, shadow = look("default", "corner_radius = 20\nshadow = 24\n")
assert comp.stop()
assert sum(shadow["side"]) < sum(BACKGROUND), shadow
print("frames OK")
