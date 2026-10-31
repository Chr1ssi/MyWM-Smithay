"""The login screen in the cyberdeck style: every third row of the background is a faint dark line (scanlines),
which the default style does not draw, and the card carries the console tag.

Needs grim and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/greeter_style_smoke.py
"""
import os, subprocess, sys, tempfile, time
from smoke_support import Compositor, wait

base = tempfile.mkdtemp()
sessions = f"{base}/sessions"
os.makedirs(sessions)
with open(f"{sessions}/splice.desktop", "w") as f:
    f.write("[Desktop Entry]\nName=Splice\nExec=/run/splice-launch\nType=Application\n")
greetd_log = f"{base}/greetd"
open(greetd_log, "w").close()
greetd = subprocess.Popen([sys.executable, "tests/fake_greetd.py", f"{base}/greetd.sock", "tester", "geheim", greetd_log])
env = {"GREETD_SOCK": f"{base}/greetd.sock", "SPLICE_GREETER_USER": "tester", "SPLICE_GREETER_SESSIONS": sessions,
       "LANG": "en_US.UTF-8", "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent", "PIPEWIRE_RUNTIME_DIR": "/nonexistent"}


def rows(comp, count=9):
    """The red channel of `count` rows of the background at the top left, where nothing else is drawn."""
    shot = subprocess.run(["grim", "-g", f"5,100 1x{count}", "-t", "ppm", "-"], env=comp.env(), capture_output=True,
                          check=True, timeout=15).stdout
    pixels = shot[-3 * count:]
    return [pixels[i * 3] for i in range(count)]


def greeter(style):
    comp = Compositor(config=f'[appearance]\nstyle = "{style}"\n', extra_env=env, argv=["--greeter"])
    try:
        # The card shows once the login screen is drawn: the avatar's pixel differs from the background.
        assert wait(lambda: subprocess.run(["grim", "-g", "640,480 1x1", "-t", "ppm", "-"], env=comp.env(),
                                           capture_output=True).stdout[-3:] != subprocess.run(
            ["grim", "-g", "5,795 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:], 15), "no card"
        time.sleep(1)
        return rows(comp)
    except BaseException:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
        raise
    finally:
        if comp.process.poll() is None:
            comp.stop()


try:
    assert wait(lambda: "ready" in open(greetd_log).read().splitlines()), "the stand-in for greetd did not start"
    plain = greeter("default")
    assert len(set(plain)) == 1, plain
    deck = greeter("cyberdeck")
    # Every third row of the screen is a line, counted from the top: of rows 100 to 108 those with a number divisible by 3.
    lines = [deck[i] for i in range(9) if (100 + i) % 3 == 0]
    others = [deck[i] for i in range(9) if (100 + i) % 3 != 0]
    assert max(lines) < min(others), deck
    assert min(others) == plain[0], (deck, plain)
    print("scanlines OK", deck)
finally:
    greetd.terminate()
