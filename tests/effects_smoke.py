"""Rounded corners (shader) and opacity against the nested compositor, checked with grim.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/effects_smoke.py
"""
import subprocess, time
from smoke_support import Compositor

BACKGROUND, ACTIVE = "#1E1E2E", "#89B4FA"


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()


def shot(comp, name):
    path = f"{comp.dir}/{name}.png"
    subprocess.run(["grim", path], env=comp.env(), check=True, timeout=15)
    return path


def run(config, windows=1):
    comp = Compositor(config=config, windows=windows)
    try:
        time.sleep(3)
        return comp, shot(comp, "shot")
    except BaseException:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
        raise


# Square corners by default: the window's corner pixel is not background.
comp, path = run("")
assert pixel(path, 11, 11) != BACKGROUND, pixel(path, 11, 11)
assert comp.stop()

# Rounded: the corner shows the background, the ring edge the border color, the middle the window.
comp, path = run("[effects]\ncorner_radius = 30\n")
try:
    assert pixel(path, 11, 11) == BACKGROUND, pixel(path, 11, 11)
    assert pixel(path, 40, 9) == ACTIVE, pixel(path, 40, 9)
    assert pixel(path, 640, 400) not in (BACKGROUND, ACTIVE)
    # A straight edge far from the corners stays intact.
    assert pixel(path, 640, 11) != BACKGROUND
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# Opacity from a rule blends the window with the dark background: its average brightness drops.
def brightness(path):
    out = subprocess.check_output(["convert", path, "-crop", "600x400+340+200", "-resize", "1x1!", "-colorspace", "Gray", "-format", "%[fx:mean]", "info:"])
    return float(out.decode())


comp, path = run("")
plain = brightness(path)
assert comp.stop()
comp, path = run("[[rules]]\napp_id = 'org.freedesktop.weston.simple-shm'\nopacity = 0.4\n")
faded = brightness(path)
assert comp.stop()
print("brightness", plain, faded)
assert faded < plain * 0.8, (plain, faded)
print("OK")
