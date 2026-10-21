"""The built-in shell's media panel against a stand-in player (tests/fake_mpris.py) on a private session bus: the bar's
media button opens it in the middle below the bar with the cover, the buttons pause, play and skip, and a button the
player does not allow does nothing. The media button shows only while something plays.

Needs dbus-daemon, python3 with dbus and gi, and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/media_smoke.py
"""
import os, shutil, struct, subprocess, sys, tempfile, time, zlib
from smoke_support import Compositor, private_bus, wait

for tool in ("dbus-daemon", "grim"):
    if not shutil.which(tool):
        print(f"SKIP: {tool} is not installed")
        sys.exit(0)
try:
    import dbus, gi  # noqa: F401  (the stand-in needs them)
except ImportError:
    print("SKIP: python3 without dbus and gi")
    sys.exit(0)

BACKGROUND = bytes.fromhex("1E1E2E")
RED = bytes.fromhex("FF0000")
PANEL = ((1280 - 400) // 2, 46)
# On 1280x800 with DejaVu Sans, the panel's title being 23 px high: the player's card starts below it, and the cover
# lies 14 inside the card (150 high, 332 wide: the card's width less twice its padding).
CARD = 46 + 20 + 23 + 14
COVER_MIDDLE = CARD + 14 + 75
COVER_LEFT, COVER_RIGHT = PANEL[0] + 20 + 14 + 2, PANEL[0] + 400 - 20 - 14 - 3
# The buttons' row, in the middle of the play button: the cover and the title (19) and artist (16) lines.
BUTTONS = CARD + 14 + 150 + 8 + 19 + 2 + 16 + 8 + 23
MEDIA_BUTTON = (520, 22)  # the bar's media button, left of the clock


def png(path, width, height, rgb):
    """A plain PNG of one color, without an image library."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    rows = b"".join(b"\0" + rgb * width for _ in range(height))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


work = tempfile.mkdtemp()
os.makedirs(f"{work}/session")
bus_command, address = private_bus(f"{work}/session")
processes = [subprocess.Popen(bus_command)]
assert wait(lambda: os.path.exists(f"{work}/session/bus")), "no bus"
env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address)
png(f"{work}/cover.png", 64, 32, RED)
calls = f"{work}/calls"
open(calls, "w").close()


def region(comp, x, y, w, h):
    shot = subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True, timeout=15).stdout
    return shot[-w * h * 3:]


def empty(comp, x, y, w, h):
    data = region(comp, x, y, w, h)
    return all(data[i:i + 3] == BACKGROUND for i in range(0, len(data), 3))


def click(x, y):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)


def called():
    return open(calls).read().split()


comp = None
try:
    processes.append(subprocess.Popen(["python3", "tests/fake_mpris.py", "Testlied", "Testband", f"file://{work}/cover.png", calls], env=env))
    extra = {"DBUS_SESSION_BUS_ADDRESS": address, "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent", "PIPEWIRE_RUNTIME_DIR": "/nonexistent"}
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    subprocess.run(["xdotool", "mousemove", "300", "500"], check=True)
    # The media button shows the playing title.
    assert wait(lambda: not empty(comp, 500, 12, 40, 20), 20), "no media button"
    click(*MEDIA_BUTTON)
    # The panel in the middle, with the cover across its width.
    assert wait(lambda: region(comp, 640, COVER_MIDDLE, 1, 1) == RED, 10), region(comp, 640, COVER_MIDDLE, 1, 1).hex()
    assert region(comp, COVER_LEFT, COVER_MIDDLE, 1, 1) == RED and region(comp, COVER_RIGHT, COVER_MIDDLE, 1, 1) == RED
    print("media panel with cover OK")

    # Play/pause in the middle of the row; paused, the bar's button goes.
    click(640, BUTTONS)
    assert wait(lambda: called() == ["Pause"]), called()
    assert wait(lambda: empty(comp, 500, 12, 40, 20), 5), "the media button stayed while paused"
    click(640, BUTTONS)
    assert wait(lambda: called() == ["Pause", "Play"]), called()
    # Next on the right; previous on the left is not allowed and does nothing.
    click(640 + 40, BUTTONS)
    assert wait(lambda: called() == ["Pause", "Play", "Next"]), called()
    click(640 - 40, BUTTONS)
    time.sleep(1)
    assert called() == ["Pause", "Play", "Next"], called()
    print("media buttons OK")

    # The media button closes the panel again.
    click(*MEDIA_BUTTON)
    assert wait(lambda: region(comp, 640, COVER_MIDDLE, 1, 1) != RED, 5), "the panel did not close"
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    raise
finally:
    for process in processes:
        process.kill()
