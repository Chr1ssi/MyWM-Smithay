"""The compositor's own wallpaper: spans the outputs and follows the picker's choice, fading to a new one when
animations are on.

Needs grim and ImageMagick. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/wallpaper_smoke.py
"""
import json, os, subprocess, tempfile
from smoke_support import Bar, Compositor, wait

RED, BLUE, GREEN = "#FF0000", "#0000FF", "#00FF00"


def pixel(env, output, x, y):
    with tempfile.NamedTemporaryFile(suffix=".png") as shot:
        subprocess.run(["grim", "-o", output, shot.name], env=env, check=True, timeout=15)
        out = subprocess.check_output(["convert", shot.name, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()[:7]


def mixed(color):
    """Neither the green nor the red wallpaper: a blend of both."""
    red, green = int(color[1:3], 16), int(color[3:5], 16)
    return 0 < red < 255 and 0 < green < 255


images = tempfile.mkdtemp()
split, green = f"{images}/split #1.png", f"{images}/green.png"
# Left half red, right half blue: on two outputs side by side, each shows one half.
subprocess.run(["convert", "-size", "32x32", "xc:red", "xc:blue", "+append", split], check=True)
subprocess.run(["convert", "-size", "64x32", "xc:#00ff00", green], check=True)


def choose(state_home, path):
    """What the picker saves (a percent-encoded file URL)."""
    os.makedirs(f"{state_home}/splice", exist_ok=True)
    url = "file://" + "".join(c if c.isalnum() or c in "/-_.~" else "%%%02X" % ord(c) for c in path)
    with open(f"{state_home}/splice/wallpaper.json", "w") as f:
        json.dump({"version": 1, "wallpaper": url}, f)


# The state directory is the compositor's own; the choice must be there before it starts.
comp = Compositor(extra_env={"SPLICE_VIRTUAL_OUTPUTS": "1"}, top=f'wallpaper_directory = "{images}"\n')
try:
    # Without a saved choice: the first image of the directory (sorted, "green" < "split").
    env = comp.env()
    assert wait(lambda: pixel(env, "winit", 640, 400) == GREEN, 15), pixel(env, "winit", 640, 400)

    # The picker saves a choice and asks for a theme reload; the picture changes on both outputs.
    choose(comp.dir + "/state", split)
    bar = Bar(comp.sock)
    assert bar.send("theme-reload") == "v1 ok"
    assert wait(lambda: pixel(env, "winit", 640, 400) == RED, 15), pixel(env, "winit", 640, 400)
    assert pixel(env, "virtual-1", 640, 400) == BLUE, pixel(env, "virtual-1", 640, 400)
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# With animations the new picture fades in over the old one: a blend shows, then only the new one.
only_green = tempfile.mkdtemp()
subprocess.run(["cp", green, only_green], check=True)
red_image = f"{images}/red.png"
subprocess.run(["convert", "-size", "64x32", "xc:#ff0000", red_image], check=True)
comp = Compositor(config="[effects]\nanimation_ms = 100\n", top=f'wallpaper_directory = "{only_green}"\n')
try:
    env = comp.env()
    assert wait(lambda: pixel(env, "winit", 640, 400) == GREEN, 15), pixel(env, "winit", 640, 400)
    choose(comp.dir + "/state", red_image)
    assert Bar(comp.sock).send("theme-reload") == "v1 ok"
    seen = []
    assert wait(lambda: seen.append(pixel(env, "winit", 640, 400)) or seen[-1] == RED, 15), seen
    print("fade samples", seen)
    assert any(mixed(color) for color in seen), seen
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
