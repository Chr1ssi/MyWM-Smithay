"""The built-in shell's tray against a stand-in item (tests/fake_tray.py) on a private session bus: the shell is the
watcher, the item's icon shows in the bar, the buttons activate it, the wheel scrolls it, the right button opens its
menu (without the hidden entry), a submenu leads in and back out, an entry is clicked, and the icon goes with the item.

Needs dbus-daemon, python3 with dbus and gi, and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/tray_smoke.py
"""
import os, shutil, subprocess, sys, tempfile, time
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
GREEN = bytes.fromhex("00FF00")
MENU = (1280 - 8 - 320, 46)
FIRST_ENTRY = MENU[1] + 12 + 20 + 4 + 17  # the middle of the first entry, below a title of 20
SUBMENU_ENTRY = FIRST_ENTRY + 34 + 2 + 9 + 2  # after the first entry and the separator

work = tempfile.mkdtemp()
os.makedirs(f"{work}/session")
bus_command, address = private_bus(f"{work}/session")
processes = [subprocess.Popen(bus_command)]
assert wait(lambda: os.path.exists(f"{work}/session/bus")), "no bus"
env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address)
calls = f"{work}/calls"
open(calls, "w").close()


def row(comp, y):
    shot = subprocess.run(["grim", "-g", f"0,{y} 1280x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True, timeout=15).stdout
    data = shot[-1280 * 3:]
    return [data[i * 3:i * 3 + 3] for i in range(1280)]


def icon_x(comp):
    """Where the green icon is in the bar, if it shows."""
    pixels = row(comp, 22)
    found = [x for x, p in enumerate(pixels) if p == GREEN]
    return found[len(found) // 2] if found else None


def empty(comp, x, y, w, h):
    shot = subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True, timeout=15).stdout
    data = shot[-w * h * 3:]
    return all(data[i:i + 3] == BACKGROUND for i in range(0, len(data), 3))


def menu_shows(comp):
    return not empty(comp, MENU[0] + 12, MENU[1] + 12, 100, 18)


def button(x, y, number):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", str(number)], check=True)


def called():
    return open(calls).read().splitlines()


comp = None
try:
    extra = {"DBUS_SESSION_BUS_ADDRESS": address, "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent", "PIPEWIRE_RUNTIME_DIR": "/nonexistent"}
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    item = subprocess.Popen(["python3", "tests/fake_tray.py", calls], env=env)
    processes.append(item)
    subprocess.run(["xdotool", "mousemove", "300", "500"], check=True)
    assert wait(lambda: icon_x(comp) is not None, 20), "the tray icon did not show"
    x = icon_x(comp)
    print("tray icon shows OK")

    button(x, 22, 1)
    assert wait(lambda: "Activate" in called()), called()
    button(x, 22, 2)
    assert wait(lambda: "SecondaryActivate" in called()), called()
    button(x, 22, 4)
    assert wait(lambda: "Scroll 120 vertical" in called()), called()
    print("activate and scroll OK")

    # The menu: "Öffnen", a separator, "Mehr"; the hidden entry is not there.
    button(x, 22, 3)
    assert wait(lambda: menu_shows(comp), 10), "the menu did not open"
    assert "AboutToShow 0" in called(), called()
    assert not empty(comp, MENU[0] + 40, FIRST_ENTRY - 8, 120, 16)
    assert not empty(comp, MENU[0] + 40, SUBMENU_ENTRY - 8, 120, 16)
    # With the hidden entry the menu would reach 36 pixels further down, below its last entry.
    assert empty(comp, MENU[0] + 40, SUBMENU_ENTRY + 17 + 12 + 8, 120, 16), "the hidden entry shows"
    # Into the submenu (the back button makes the header 26 high) and click its entry.
    button(MENU[0] + 100, SUBMENU_ENTRY, 1)
    assert wait(lambda: "AboutToShow 3" in called()), called()
    button(MENU[0] + 100, FIRST_ENTRY + 6, 1)
    assert wait(lambda: "Event 4 clicked" in called()), called()
    assert wait(lambda: not menu_shows(comp), 5), "the menu stayed after a click"
    # Again, and the top entry.
    button(x, 22, 3)
    assert wait(lambda: menu_shows(comp), 10)
    button(MENU[0] + 100, FIRST_ENTRY, 1)
    assert wait(lambda: "Event 1 clicked" in called()), called()
    print("menu and submenu OK")

    # The item ends: its icon goes.
    item.kill()
    assert wait(lambda: icon_x(comp) is None, 10), "the icon stayed after the item ended"
    print("item goes OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    print("calls:", called())
    raise
finally:
    for process in processes:
        process.kill()
