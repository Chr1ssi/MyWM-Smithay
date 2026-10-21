"""The built-in shell's wallpaper picker (as mywm-shell's test of the Quickshell picker): the wallpaper binding opens
it on the image in use, the filter narrows the images, Return saves the choice as a file URL once the image loads
(the wallpaper follows, the theme is generated from it) and closes; an image that does not load is refused and the
picker stays; no match saves nothing; Escape and a click beside the tiles close it; a click on a neighbour selects
it, on the selected one chooses it; the wheel and the held arrow move through the images and stop at the last.

Needs grim. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/wallpaper_picker_smoke.py
"""
import json, os, shutil, struct, subprocess, sys, tempfile, time, zlib
from smoke_support import Compositor, wait

if not shutil.which("grim"):
    print("SKIP: grim is not installed")
    sys.exit(0)

# The output is 1280x800; the picker's column starts at y = 68, the carousel 44 below.
SELECTED = (640, 344)
# The left neighbour, where the selected tile does not cover it.
NEIGHBOUR = (210, 344)
BESIDE = (640, 785)
# The wallpaper shows here: the gradient's (1, 143, 100) or the second image's (0, 200, 0); dimmed, a third of it.
CORNER = (5, 795)


def png(path, w, h, color):
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(v for x in range(w) for v in color(x, y)) for y in range(h))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


images = tempfile.mkdtemp()
png(f"{images}/a gradient # 100%.png", 256, 144, lambda x, y: (x, y, 100))
png(f"{images}/b second.PNG", 64, 36, lambda x, y: (0, 200, 0))
with open(f"{images}/c broken.png", "wb") as f:
    f.write(b"not a picture")
with open(f"{images}/ignored.txt", "w") as f:
    f.write("not an image")


def pixel(comp, x, y):
    return subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:]


def window():
    return subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()


def type_text(text):
    subprocess.run(["xdotool", "windowfocus", window()], check=True)
    subprocess.run(["xdotool", "type", "--delay", "60", text], check=True)
    time.sleep(0.3)


def click(x, y, button="1"):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", button], check=True)
    time.sleep(0.3)


comp = None
try:
    extra = {"DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent", "PIPEWIRE_RUNTIME_DIR": "/nonexistent", "LANG": "de_DE.UTF-8"}
    comp = Compositor(top=f'builtin_shell = true\nwallpaper_directory = "{images}"\n', extra_env=extra)
    state = f"{comp.dir}/state/mywm"
    is_closed = lambda: max(pixel(comp, *CORNER)) > 120
    assert wait(is_closed, 20), "the wallpaper was not drawn"

    def open_picker():
        subprocess.run(["xdotool", "mousemove", *map(str, BESIDE)], check=True)
        comp.key("alt+shift+w")
        assert wait(lambda: max(pixel(comp, *CORNER)) < 90), "the picker did not open"
        # The pictures load in the background (a choice waits for its own).
        time.sleep(1)

    def saved():
        try:
            return json.load(open(f"{state}/wallpaper.json"))["wallpaper"]
        except (OSError, ValueError, KeyError):
            return None

    # The filter; Return saves the choice and closes; the wallpaper and the theme follow it.
    open_picker()
    type_text("SECOND")
    comp.key("Return")
    assert wait(lambda: (saved() or "").endswith("/b%20second.PNG")), saved()
    assert json.load(open(f"{state}/wallpaper.json"))["version"] == 1
    assert wait(is_closed), "the picker stayed open"
    assert wait(lambda: pixel(comp, 640, 400) == bytes.fromhex("00c800"), 15), pixel(comp, 640, 400)

    def theme_source():
        try:
            return json.load(open(f"{state}/theme.json"))["source"]
        except (OSError, ValueError, KeyError):
            return ""
    assert wait(lambda: theme_source().endswith("/b second.PNG"), 20), theme_source()
    print("filter, save, wallpaper and theme OK")

    # It opens on the image in use (the second): the next one does not load, so it is refused and the picker stays.
    open_picker()
    comp.key("Right")
    comp.key("Return")
    time.sleep(1.5)
    assert not is_closed(), "a broken image was accepted"
    assert saved().endswith("/b%20second.PNG"), saved()
    comp.key("Escape")
    assert wait(is_closed), "Escape did not close"
    print("current image, broken image, Escape OK")

    # No match: Return saves nothing and the picker stays.
    open_picker()
    type_text("no matches")
    comp.key("Return")
    time.sleep(0.5)
    assert not is_closed()
    comp.key("Escape")
    assert wait(is_closed)

    # A click beside the tiles closes it.
    open_picker()
    click(*BESIDE)
    assert wait(is_closed), "the click beside did not close"

    # A click on the neighbour selects it, a second one (now it is the selected tile) chooses it.
    open_picker()
    click(*NEIGHBOUR)
    time.sleep(0.5)
    assert not is_closed()
    click(*SELECTED)
    assert wait(lambda: "%23" in (saved() or "") and "%25" in saved()), saved()
    assert saved().endswith("/a%20gradient%20%23%20100%25.png"), saved()
    assert wait(is_closed)
    print("clicks OK")

    # The held arrow stops at the last image (the broken one); one back is the second.
    open_picker()
    subprocess.run(["xdotool", "keydown", "Right"], check=True)
    time.sleep(1.5)
    subprocess.run(["xdotool", "keyup", "Right"], check=True)
    comp.key("Left")
    comp.key("Return")
    assert wait(lambda: (saved() or "").endswith("/b%20second.PNG")), saved()
    assert wait(is_closed)

    # The wheel over the tiles: down is the next one (the broken one again).
    open_picker()
    subprocess.run(["xdotool", "mousemove", *map(str, SELECTED)], check=True)
    click(*SELECTED, button="5")
    comp.key("Return")
    time.sleep(1.5)
    assert not is_closed(), "the wheel did not move to the broken image"
    comp.key("Escape")
    assert wait(is_closed)
    print("held arrow and wheel OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    raise
