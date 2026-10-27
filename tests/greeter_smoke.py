"""The built-in login screen (`splice-compositor --greeter` without a command; as splice-shell's test of greeter.qml)
against a stand-in for greetd (`tests/fake_greetd.py`): the card shows on the primary monitor only, bindings do
nothing, a wrong password is refused (the field turns red), Tab and Return pick the second session, the right
password starts it (field codes left out) and the compositor ends with code 0, as greetd needs. The shut down button
runs `systemctl poweroff` (a stand-in here). Without a preset user the user field has the keyboard and Return moves
on to the password. A greetd that cannot be reached is an error that keeps the login screen.

Needs grim and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/greeter_smoke.py
"""
import os, subprocess, sys, tempfile, time
from smoke_support import Compositor, wait

USER, PASSWORD = "tester", "geheim"
# On the 1280x800 output the card begins at y = 416 (52 %): avatar, user field (532-580), password field (596-644).
AVATAR = (640, 480)
PASSWORD_FIELD = (640, 620)
PASSWORD_BORDER = (469, 620)
# The shut down button, the last of the island 24 px from the bottom right corner.
POWEROFF = (1238, 760)
# The theme's alert color without a theme.json.
ALERT = bytes.fromhex("f38ba8")


def pixel(comp, x, y):
    return subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:]


def colors(comp, output):
    shot = subprocess.run(["grim", "-o", output, "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True).stdout
    # Binary PPM: "P6", width, height, maxval, then RGB triples.
    pixels = shot.split(maxsplit=4)[4]
    return len({pixels[i:i + 3] for i in range(0, len(pixels) - 2, 3)})


def type_text(text):
    window = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    subprocess.run(["xdotool", "windowfocus", window], check=True)
    subprocess.run(["xdotool", "type", "--delay", "60", text], check=True)
    time.sleep(0.3)


def click(x, y):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)
    time.sleep(0.3)


base = tempfile.mkdtemp()
sessions = f"{base}/sessions"
os.makedirs(sessions)
with open(f"{sessions}/splice.desktop", "w") as f:
    f.write("[Desktop Entry]\nName=splice\nExec=/run/splice-launch\nType=Application\n")
with open(f"{sessions}/sway.desktop", "w") as f:
    f.write("[Desktop Entry]\nName=Andere\nExec=/run/other-launch --flag %U\n")
# Restart and shut down must never reach the real system.
os.makedirs(f"{base}/bin")
with open(f"{base}/bin/systemctl", "w") as f:
    f.write(f"#!/usr/bin/env bash\necho \"$@\" >> {base}/systemctl\n")
os.chmod(f"{base}/bin/systemctl", 0o755)
greetd_log = f"{base}/greetd"
open(greetd_log, "w").close()
greetd = subprocess.Popen([sys.executable, "tests/fake_greetd.py", f"{base}/greetd.sock", USER, PASSWORD, greetd_log])
requests = lambda: open(greetd_log).read().splitlines()
marker = f"{base}/binding"
comp = None
try:
    assert wait(lambda: "ready" in requests()), "the stand-in for greetd did not start"
    env = {"GREETD_SOCK": f"{base}/greetd.sock", "SPLICE_GREETER_USER": USER, "SPLICE_GREETER_SESSIONS": sessions,
           "LANG": "en_US.UTF-8", "SPLICE_VIRTUAL_OUTPUTS": "1", "PATH": f"{base}/bin:{os.environ['PATH']}",
           "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent", "PIPEWIRE_RUNTIME_DIR": "/nonexistent"}
    config = f'[program_bindings.marker]\nkeys = ["XF86AudioRaiseVolume"]\ncommand = ["touch", "{marker}"]\n'
    comp = Compositor(config=config, extra_env=env, argv=["--greeter"])
    background = lambda: pixel(comp, 5, 795)
    assert wait(lambda: pixel(comp, *AVATAR) != background(), 15), "the login card does not show"
    time.sleep(0.5)
    # The card and the islands are on the primary monitor only; the other shows the background alone.
    assert colors(comp, "virtual-1") == 1, "the second monitor shows more than the background"
    assert colors(comp, "winit") > 100, "no login card on the primary monitor"
    comp.key("XF86AudioRaiseVolume")
    time.sleep(0.5)
    assert not os.path.exists(marker), "a binding ran on the login screen"
    print("login screen OK")

    # The user is preset, so the password has the keyboard.
    type_text("falsch")
    comp.key("Return")
    assert wait(lambda: "response wrong" in requests(), 10), requests()
    assert requests()[1:3] == [f"create_session {USER}", "response wrong"], requests()
    assert wait(lambda: pixel(comp, *PASSWORD_BORDER) == ALERT, 5), pixel(comp, *PASSWORD_BORDER).hex()
    print("wrong password OK")

    # Tab moves from the password to the sessions; Return picks the second (sorted by file name: sway.desktop).
    comp.key("Tab")
    comp.key("Tab")
    comp.key("Return")
    click(*PASSWORD_FIELD)
    type_text(PASSWORD)
    comp.key("Return")
    assert wait(lambda: "start_session /run/other-launch --flag" in requests(), 10), requests()
    # greetd starts the session once the greeter is gone: the compositor ends.
    assert wait(lambda: comp.process.poll() is not None, 10), "the compositor did not end"
    assert comp.process.returncode == 0, comp.process.returncode
    print("login OK")

    # The shut down button, on a fresh login screen without a preset user.
    fresh = {k: v for k, v in env.items() if k != "SPLICE_GREETER_USER"}
    comp = Compositor(extra_env=fresh, argv=["--greeter"])
    assert wait(lambda: pixel(comp, *AVATAR) != pixel(comp, 5, 795), 15), "the login card does not show"
    time.sleep(0.5)
    click(*POWEROFF)
    assert wait(lambda: os.path.exists(f"{base}/systemctl") and open(f"{base}/systemctl").read() == "poweroff\n", 5), \
        "shut down did not run systemctl poweroff"
    print("power OK")
    # The user field has the keyboard; Return goes on to the password; the first session is the one chosen.
    type_text("testerx")
    comp.key("BackSpace")
    comp.key("Return")
    type_text(PASSWORD)
    comp.key("Return")
    assert wait(lambda: "start_session /run/splice-launch" in requests(), 10), requests()
    assert f"create_session {USER}" in requests()[-3:], requests()
    assert wait(lambda: comp.process.poll() is not None, 10) and comp.process.returncode == 0, "the compositor did not end"
    print("user field OK")

    # greetd cannot be reached: an error, and the login screen stays.
    comp = Compositor(extra_env=dict(env, GREETD_SOCK=f"{base}/nonexistent"), argv=["--greeter"])
    assert wait(lambda: pixel(comp, *AVATAR) != pixel(comp, 5, 795), 15), "the login card does not show"
    time.sleep(0.5)
    type_text(PASSWORD)
    comp.key("Return")
    assert wait(lambda: pixel(comp, *PASSWORD_BORDER) == ALERT, 10), pixel(comp, *PASSWORD_BORDER).hex()
    assert comp.process.poll() is None, "the login screen ended without a session"
    assert comp.stop()
    print("unreachable greetd OK")
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-4000:])
    raise
finally:
    greetd.terminate()
