"""The built-in shell's lock screen (as mywm-shell's test of lock.qml): `--lock` gets its confirmation once the lock
screen shows, nothing behind it shows (in a capture of the monitor neither) or gets a key, a wrong password keeps the
session locked, so does a check that dies on the way and a screen locker that asks to take over and to unlock; the
right password unlocks (Return, or a click on the button). The card shows on the monitor; a refused password turns
the field's border red. The window under a pointer that does not move loses it when the session locks; the theme's
image, read when locking, shows dimmed behind the card. PAM runs with its own service directory (pam_exec), never a real account.

Needs grim, `cargo build --workspace` and $MYWM_TEST_PAM_MODULES (the smoke shell sets it). Run from the repository
root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/lock_screen_smoke.py
"""
import json, os, struct, subprocess, sys, tempfile, time, zlib
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/mywm-test-client"
PASSWORD = "geheim"
KEY_A = 30
# On the 1280x800 output the card begins at y = 416 (52 %): avatar at its top, then name and the field (y 575-623).
AVATAR = (640, 480)
FIELD_BORDER = (469, 599)
SUBMIT = (787, 599)
# The theme's alert color without a theme.json.
ALERT = bytes.fromhex("f38ba8")

if not os.environ.get("MYWM_TEST_PAM_MODULES"):
    print("SKIP: $MYWM_TEST_PAM_MODULES is not set (run in the smoke shell)")
    sys.exit(0)


def pam_directory(base):
    """A PAM service `mywm-lock` that accepts PASSWORD only; `abbruch` kills the check midway."""
    modules = os.environ["MYWM_TEST_PAM_MODULES"]
    check = f"{base}/check-password"
    with open(check, "w") as f:
        # pam_exec hands the password over on stdin, ended by a NUL; its parent is the compositor's check.
        f.write("#!/usr/bin/env bash\nIFS= read -r -d '' password\n"
                "[ \"$password\" = abbruch ] && kill -KILL $PPID\n"
                f"[ \"$password\" = {PASSWORD} ]\n")
    os.chmod(check, 0o755)
    os.makedirs(f"{base}/pam.d")
    with open(f"{base}/pam.d/mywm-lock", "w") as f:
        f.write(f"auth required {modules}/pam_exec.so expose_authtok quiet {check}\n"
                f"account required {modules}/pam_permit.so\n")
    return f"{base}/pam.d"


def png(path, w, h, color):
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(v for x in range(w) for v in color) for y in range(h))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def pixel(comp, x, y):
    return subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:]


def type_text(text):
    window = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    subprocess.run(["xdotool", "windowfocus", window], check=True)
    subprocess.run(["xdotool", "type", "--delay", "60", text], check=True)
    time.sleep(0.3)


base = tempfile.mkdtemp()
comp = None
try:
    extra = {"MYWM_PAM_DIR": pam_directory(base), "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
             "PIPEWIRE_RUNTIME_DIR": "/nonexistent", "LANG": "de_DE.UTF-8"}
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    log = lambda: open(comp.dir + "/log").read()
    keys = open(comp.dir + "/client", "w")
    subprocess.Popen([CLIENT, "keys", "60"], env=comp.env(), stdout=keys)
    client = lambda: open(comp.dir + "/client").read()
    assert wait(lambda: "keys client ready" in client(), 10), "the keys client did not start"
    time.sleep(1)
    comp.key("a")
    assert wait(lambda: f"key {KEY_A} released" in client(), 5), "the window gets no keys: " + client()
    # The window's 200x150 picture, at the top left of its tile, and the desktop's background.
    window = pixel(comp, 100, 120)
    corner = pixel(comp, 5, 795)
    print("before the lock:", window.hex(), corner.hex())
    assert window == bytes.fromhex("808080"), "the window is not where it should be"

    # Locking: confirmed once the lock screen shows; it covers everything, captures included.
    result = subprocess.run(["./target/debug/mywm-compositor", "--lock"], env=comp.env(), timeout=20)
    assert result.returncode == 0, "--lock got no confirmation"
    bar.until(lambda s: bar.locked, 5)
    shown = pixel(comp, 100, 120)
    print("locked:", shown.hex())
    assert shown != window, "the window shows behind the lock"
    assert shown == pixel(comp, 5, 795) == pixel(comp, 1275, 5), "the lock screen does not cover everything"
    assert wait(lambda: pixel(comp, *AVATAR) != shown, 10), "the card does not show"
    field_border = pixel(comp, *FIELD_BORDER)
    assert field_border != ALERT, field_border.hex()
    assert bar.send("workspace 1 1") == "v1 error invalid-command", "commands must be refused while locked"
    print("lock OK")

    # A wrong password: still locked, and the window got none of the keys.
    seen = client()
    type_text("falsch")
    comp.key("Return")
    assert wait(lambda: "password was not accepted" in log(), 10), "the wrong password was not checked"
    time.sleep(0.5)
    bar.poll()
    assert bar.locked, "a wrong password unlocked"
    assert client() == seen, "keys reached the window behind the lock:\n" + client()[len(seen):]
    assert wait(lambda: pixel(comp, *FIELD_BORDER) == ALERT, 5), pixel(comp, *FIELD_BORDER).hex()
    print("wrong password OK")

    # A check that dies on the way unlocks nothing.
    type_text("abbruch")
    comp.key("Return")
    assert wait(lambda: log().count("password was not accepted") == 2, 10), "the dead check did not end"
    assert "the password check failed" in log()
    bar.poll()
    assert bar.locked, "a check that died unlocked"
    print("dead check OK")

    # Another screen locker is refused and cannot end the lock.
    locker = subprocess.run([CLIENT, "lock", "1"], env=comp.env(), capture_output=True, text=True, timeout=20)
    assert locker.stdout.splitlines()[:1] == ["finished"], locker.stdout + locker.stderr
    time.sleep(0.5)
    bar.poll()
    assert bar.locked, "a screen locker ended the built-in lock"
    assert pixel(comp, 100, 120) == shown, "a screen locker changed the lock screen"
    print("foreign locker OK")

    # The right password and a click on the button unlock: the window shows and gets keys again.
    type_text(PASSWORD)
    subprocess.run(["xdotool", "mousemove", *map(str, SUBMIT), "click", "1"], check=True)
    bar.until(lambda s: not bar.locked, 10)
    assert wait(lambda: pixel(comp, 100, 120) == window and pixel(comp, 5, 795) == corner, 5), "the desktop did not come back"
    assert client() == seen, "keys reached the window behind the lock:\n" + client()[len(seen):]
    comp.key("a")
    assert wait(lambda: client().count(f"key {KEY_A} released") == 2, 5), "the window gets no keys after unlocking"
    print("unlock OK")
    assert comp.stop()

    # Locking without moving the pointer takes it from the window under it (else the wheel would reach it).
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    Bar(comp.sock).until(lambda s: len(s) == 1)
    # Written after the start: the lock reads the theme anew.
    png(f"{base}/background.png", 64, 40, (200, 100, 50))
    os.makedirs(f"{comp.dir}/state/mywm", exist_ok=True)
    with open(f"{comp.dir}/state/mywm/theme.json", "w") as f:
        json.dump({"version": 1, "source": f"{base}/background.png", "colors": {"primary": {"default": {"hex": "#00ff00"}}}}, f)
    events = open(comp.dir + "/client", "w")
    subprocess.Popen([CLIENT, "pointer", "60"], env=comp.env(), stdout=events)
    client = lambda: open(comp.dir + "/client").read()
    assert wait(lambda: "pointer client ready" in client(), 10), "the pointer client did not start"
    time.sleep(1)
    subprocess.run(["xdotool", "mousemove", "100", "120"], check=True)
    assert wait(lambda: "pointer enter" in client(), 5), "the pointer did not enter the window: " + client()
    result = subprocess.run(["./target/debug/mywm-compositor", "--lock"], env=comp.env(), timeout=20)
    assert result.returncode == 0, "--lock got no confirmation"
    assert wait(lambda: "pointer leave" in client(), 5), "the window kept the pointer behind the lock: " + client()
    print("pointer OK")
    dimmed = lambda: all(abs(got - round(want * 0.65)) <= 2 for got, want in zip(pixel(comp, 5, 795), (200, 100, 50)))
    assert wait(dimmed, 10), "the theme's image does not show: " + pixel(comp, 5, 795).hex()
    print("background OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-4000:])
    raise
