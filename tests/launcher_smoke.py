"""The built-in shell's launcher with harmless test applications (as mywm-shell's test of the Quickshell launcher):
the launcher binding opens it, every word typed must match (keywords count), the selection moves with the arrows,
Return starts the application with its arguments and working directory (in the terminal when it asks for one) and
closes, no match starts nothing, Escape closes, a click on a row starts it, and a held Backspace repeats.

Needs dbus-daemon and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/launcher_smoke.py
"""
import json, os, shutil, struct, subprocess, sys, tempfile, time, zlib
from smoke_support import Compositor, wait

if not shutil.which("grim"):
    print("SKIP: grim is not installed")
    sys.exit(0)

FIRST_ROW = (320 + 16 + 100, 120 + 107 + 26)  # the card in the middle of 1280x800, 640x560
FIRST_ICON = (320 + 16 + 12 + 16, 120 + 107 + 10 + 16)
# In the search field: the card itself is glass over the same background color.
CARD = (800, 191)

base = tempfile.mkdtemp()
apps = f"{base}/data/applications"
os.makedirs(apps)
os.makedirs(f"{base}/empty")
marker = f"{base}/launched.json"
terminal_marker = f"{base}/terminal.json"
probe = f"{base}/probe"
with open(probe, "w") as f:
    f.write("#!/usr/bin/env python3\nimport json,os,sys\n"
            f"open({marker!r}, 'w').write(json.dumps([sys.argv[1:], os.getcwd(), os.environ.get('__EGL_VENDOR_LIBRARY_FILENAMES')]))\n")
terminal = f"{base}/terminal"
with open(terminal, "w") as f:
    f.write(f"#!/usr/bin/env python3\nimport json,sys\nopen({terminal_marker!r}, 'w').write(json.dumps(sys.argv[1:]))\n")
icon = f"{base}/icon.png"


def png(path, size, rgb):
    """A plain PNG of one color."""
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(rgb) * size for _ in range(size))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


png(icon, 48, (255, 0, 0))
for script in (probe, terminal):
    os.chmod(script, 0o755)
with open(f"{apps}/probe-browser.desktop", "w") as f:
    f.write(f"[Desktop Entry]\nType=Application\nName=Probe Browser\nGenericName=Web Browser\nComment=Browse test pages\n"
            f"Keywords=internet;web;\nExec={probe} \"two words\" %U\nPath={base}\nIcon={icon}\n")
with open(f"{apps}/probe-terminal.desktop", "w") as f:
    f.write(f"[Desktop Entry]\nType=Application\nName=Probe Terminal\nExec={probe}\nTerminal=true\n")
with open(f"{apps}/hidden.desktop", "w") as f:
    f.write(f"[Desktop Entry]\nType=Application\nName=Hidden Probe\nExec={probe}\nNoDisplay=true\n")


def pixel(comp, x, y):
    return subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:]


def window():
    return subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()


def type_text(text):
    subprocess.run(["xdotool", "windowfocus", window()], check=True)
    subprocess.run(["xdotool", "type", "--delay", "60", text], check=True)
    time.sleep(0.3)


def launched(path):
    if not os.path.exists(path):
        return None
    try:
        return json.loads(open(path).read())
    except ValueError:
        return None


comp = None
try:
    extra = {"DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent", "PIPEWIRE_RUNTIME_DIR": "/nonexistent",
             "XDG_DATA_HOME": f"{base}/data", "XDG_DATA_DIRS": f"{base}/empty", "LANG": "de_DE.UTF-8"}
    comp = Compositor(top=f'builtin_shell = true\nterminal = ["{terminal}", "--test-arg"]\n', extra_env=extra)
    assert wait(lambda: pixel(comp, 15, 22) == bytes.fromhex("313244"), 20), "the bar was not drawn"
    closed = pixel(comp, *CARD)

    def open_launcher():
        comp.key("alt+space")
        assert wait(lambda: pixel(comp, *CARD) != closed), "the launcher did not open"
        time.sleep(0.5)

    def is_closed():
        return pixel(comp, *CARD) == closed

    # Keywords count; Return starts it with its arguments in its directory, and the launcher closes.
    open_launcher()
    type_text("INTERNET web")
    assert wait(lambda: pixel(comp, *FIRST_ICON) == bytes.fromhex("ff0000")), "no icon"
    comp.key("Return")
    assert wait(lambda: launched(marker)), "nothing started"
    assert launched(marker) == [["two words"], base, None], launched(marker)
    assert wait(is_closed), "the launcher stayed open"
    print("search and launch OK")

    # The arrow selects the second (by name); it wants a terminal.
    open_launcher()
    comp.key("Down")
    comp.key("Return")
    assert wait(lambda: launched(terminal_marker)), "the terminal did not start"
    assert launched(terminal_marker) == ["--test-arg", "-e", probe], launched(terminal_marker)
    print("terminal OK")

    # No match: Return starts nothing and the launcher stays; Escape closes it.
    os.remove(marker)
    open_launcher()
    type_text("no-such-application")
    comp.key("Return")
    time.sleep(0.5)
    assert not os.path.exists(marker) and not is_closed()
    # Held Backspace repeats and empties the field: Return then starts the first application.
    subprocess.run(["xdotool", "keydown", "BackSpace"], check=True)
    time.sleep(1.5)
    subprocess.run(["xdotool", "keyup", "BackSpace"], check=True)
    comp.key("Return")
    assert wait(lambda: launched(marker)), "the held Backspace did not empty the search"
    print("no match and key repeat OK")
    os.remove(marker)
    open_launcher()
    comp.key("Escape")
    assert wait(is_closed), "Escape did not close"

    # A click on a row starts it.
    open_launcher()
    subprocess.run(["xdotool", "mousemove", *map(str, FIRST_ROW), "click", "1"], check=True)
    assert wait(lambda: launched(marker)), "the click started nothing"
    assert wait(is_closed)
    print("click OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    raise
