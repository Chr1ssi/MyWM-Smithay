"""Rounded corners (shader) and opacity against the nested compositor, checked with grim.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/effects_smoke.py
"""
import subprocess, tempfile, time
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
def brightness(path, crop="600x400+340+200"):
    out = subprocess.check_output(["convert", path, "-crop", crop, "-resize", "1x1!", "-colorspace", "Gray", "-format", "%[fx:mean]", "info:"])
    return float(out.decode())


comp, path = run("")
plain = brightness(path)
assert comp.stop()
comp, path = run("[[rules]]\napp_id = 'splice.test.simple'\nopacity = 0.4\n")
faded = brightness(path)
assert comp.stop()
print("brightness", plain, faded)
assert faded < plain * 0.8, (plain, faded)

# `window_opacity` fades every window; a `video` rule or an `opacity = 1.0` rule keeps its window opaque.
comp, path = run("[effects]\nwindow_opacity = 0.4\n")
everything = brightness(path)
assert comp.stop()
comp, path = run("[effects]\nwindow_opacity = 0.4\n[[rules]]\napp_id = 'splice.test.simple'\nvideo = true\n")
video = brightness(path)
assert comp.stop()
comp, path = run("[effects]\nwindow_opacity = 0.4\n[[rules]]\napp_id = 'splice.test.simple'\nopacity = 1.0\n")
exempt = brightness(path)
assert comp.stop()
print("brightness", everything, video, exempt)
assert everything < plain * 0.8, (plain, everything)
assert abs(video - plain) < plain * 0.05, (plain, video)
assert abs(exempt - plain) < plain * 0.05, (plain, exempt)


# Video is recognized without a rule: by its content type, or by an idle inhibitor on the window. A `video = false`
# rule overrides that. (`frames` and `idle-inhibit` windows are not the `simple` one: compare them among themselves.)
def fade_of(client, rule="", crop="600x400+340+200"):
    comp = Compositor(config="[effects]\nwindow_opacity = 0.4\n" + rule, windows=0, extra_args=f"./target/debug/splice-test-client {client} & ")
    try:
        time.sleep(3)
        return brightness(shot(comp, "shot"), crop)
    finally:
        assert comp.stop()


opaque_frames = fade_of("frames 3600 video")
faded_frames = fade_of("frames 3600")
# The `idle-inhibit` window keeps its 200x150 buffer in the top left corner.
corner = "180x130+20+20"
opaque_inhibit = fade_of("idle-inhibit 3600", crop=corner)
faded_inhibit = fade_of("idle-inhibit 3600", "[[rules]]\napp_id = 'splice.test.0'\nvideo = false\n", corner)
print("brightness", opaque_frames, faded_frames, opaque_inhibit, faded_inhibit)
assert faded_frames < opaque_frames * 0.8, (opaque_frames, faded_frames)
assert faded_inhibit < opaque_inhibit * 0.8, (opaque_inhibit, faded_inhibit)

# Shadow: the strip next to the window is darker than the plain background.
comp, path = run("[effects]\nshadow = 30\n")
try:
    assert pixel(path, 4, 400) != BACKGROUND, pixel(path, 4, 400)
    assert pixel(path, 640, 400) != BACKGROUND
    assert comp.stop()
except BaseException:
    comp.process.kill()
    raise

# Animation: right after the window appears it is still fading in.
comp = Compositor(config="[effects]\nanimation_ms = 1000\n", windows=1)
try:
    from smoke_support import wait
    assert wait(lambda: "new window" in open(comp.dir + "/log").read(), 10)
    early = brightness(shot(comp, "early"))
    time.sleep(1.5)
    late = brightness(shot(comp, "late"))
    print("fade", early, late)
    assert early < late * 0.9, (early, late)
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# Blur: behind a nearly transparent window the wallpaper shows soft instead of sharp.
def contrast(path):
    out = subprocess.check_output(["convert", path, "-crop", "400x300+400+250", "-colorspace", "Gray", "-format", "%[fx:standard_deviation]", "info:"])
    return float(out.decode())


wallpaper = f"{tempfile.mkdtemp()}/checks.png"
subprocess.run(["convert", "-size", "40x40", "pattern:checkerboard", "-scale", "200%", "/tmp/tile.png"], check=True)
subprocess.run(["convert", "-size", "1280x800", "tile:/tmp/tile.png", wallpaper], check=True)


def blurred(amount):
    cfg = f"[effects]\nblur = {amount}\n[[rules]]\napp_id = 'splice.test.simple'\nopacity = 0.1\n"
    comp = Compositor(config=cfg, windows=1, extra_args=f"swaybg -i {wallpaper} -m fill & sleep 1; ")
    try:
        time.sleep(4)
        value = contrast(shot(comp, "blur"))
        assert comp.stop()
        return value
    except BaseException:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
        raise


sharp, soft = blurred(0), blurred(8)
print("contrast", sharp, soft)
assert soft < sharp * 0.6, (sharp, soft)
print("OK")
