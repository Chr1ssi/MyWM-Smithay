"""Built-in screenshots (screen, window, dragged region) against the nested compositor.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/screenshot_smoke.py
"""
import glob, os, subprocess, tempfile, time
from smoke_support import Compositor, wait

shots = tempfile.mkdtemp()
comp = Compositor(windows=1, top=f'screenshot_directory = "{shots}"\n')


def size(path):
    return subprocess.check_output(["identify", "-format", "%wx%h", path]).decode()


def brightness(path):
    out = subprocess.check_output(["convert", path, "-resize", "1x1!", "-colorspace", "Gray", "-format", "%[fx:mean]", "info:"])
    return float(out.decode())


def newest(known):
    assert wait(lambda: set(glob.glob(shots + "/*.png")) - known, 10), "no screenshot written"
    time.sleep(0.5)
    (path,) = set(glob.glob(shots + "/*.png")) - known
    return path


try:
    time.sleep(3)
    known = set()
    comp.key("shift+Print")
    full = newest(known)
    known.add(full)
    assert size(full) == "1280x800", size(full)
    print("screen:", size(full))
    time.sleep(1.1)  # file names have a resolution of one second

    comp.key("ctrl+Print")
    window = newest(known)
    known.add(window)
    assert size(window) == "1264x784", size(window)
    print("window:", size(window))
    time.sleep(1.1)

    # Drag a region: press Print, then drag 300x200 inside the nested window.
    win = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    comp.key("Print")
    time.sleep(0.5)
    for args in (["mousemove", "--window", win, "200", "200"], ["mousedown", "1"], ["mousemove", "--window", win, "350", "300"],
                 ["mousemove", "--window", win, "500", "400"], ["mouseup", "1"]):
        subprocess.run(["xdotool", *args], check=True)
        time.sleep(0.2)
    region = newest(known)
    assert size(region) in ("300x200", "301x201", "300x201", "301x200"), size(region)
    # The dimming overlay must not be in the picture: the window is bright, the dimmed one is ~half.
    assert brightness(region) > 0.35, brightness(region)
    print("region:", size(region), brightness(region))
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
