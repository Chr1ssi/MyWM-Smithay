"""The built-in shell (`builtin_shell = true`): the compositor draws the bar and keeps windows below it, its
buttons switch and add workspaces, the power menu opens and closes, the exit binding counts down to the
logout (Escape stops it), and a broken configuration shows a notice that its button closes.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/builtin_shell_smoke.py
"""
import os, subprocess, time
from smoke_support import Bar, Compositor, wait

BACKGROUND = "#1E1E2E"  # the default palette: the clear color and the glass of the bar
SURFACE = "#313244"  # the pill of the selected workspace
ACCENT = "#89B4FA"  # the border of the configuration notice


def pixel(comp, x, y):
    shot = subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(),
                          capture_output=True, check=True, timeout=15).stdout
    r, g, b = shot[-3:]
    return f"#{r:02X}{g:02X}{b:02X}"


def click(x, y):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)


# The shell's services process must not read the real session's buses or PipeWire.
nowhere = {"DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent", "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
           "PIPEWIRE_RUNTIME_DIR": "/nonexistent"}
comp = Compositor(top="builtin_shell = true\n", windows=1, extra_env=nowhere)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    # The fonts load in the background; then the selected workspace's pill shows.
    assert wait(lambda: pixel(comp, 15, 22) == SURFACE, 20), pixel(comp, 15, 22)
    # The window starts below the bar (40 pixels and the outer gap), not at the top.
    assert pixel(comp, 640, 44) == BACKGROUND, pixel(comp, 640, 44)
    assert pixel(comp, 640, 60) != BACKGROUND, pixel(comp, 640, 60)
    print("bar drawn, window below it OK")

    # The session's startup still runs `--bar`: with the built-in shell it starts nothing.
    started = time.time()
    helper = subprocess.run(["./target/debug/mywm-compositor", "--bar"], env=comp.env(), timeout=10)
    assert helper.returncode == 0 and time.time() - started < 5, helper.returncode
    print("--bar leaves the bar to the compositor OK")

    # "+" makes and shows a new workspace, "1" goes back.
    click(8 + 4 + 26 + 2 + 13, 22)
    bar.until(lambda s: len(s[0]["workspaces"]) == 2 and s[0]["active"] == 2)
    click(8 + 4 + 13, 22)
    bar.until(lambda s: s[0]["active"] == 1)
    print("workspace buttons OK")

    # The power button opens the menu, which dims the whole monitor; Escape closes it.
    click(1280 - 8 - 4 - 13, 22)
    assert wait(lambda: pixel(comp, 640, 44) not in (BACKGROUND,)), "the power menu did not open"
    dimmed = pixel(comp, 640, 44)
    assert int(dimmed[1:3], 16) < int(BACKGROUND[1:3], 16), dimmed
    comp.key("Escape")
    assert wait(lambda: pixel(comp, 640, 44) == BACKGROUND), "Escape did not close the power menu"
    print("power menu opens and closes OK")

    # The exit binding (Super+m, nested Alt+m) counts down to the logout; Escape stops it.
    comp.key("alt+m")
    assert wait(lambda: pixel(comp, 640, 44) != BACKGROUND), "the exit binding did not ask"
    time.sleep(1)
    comp.key("Escape")
    time.sleep(5)
    assert comp.process.poll() is None, "the logout went ahead although it was cancelled"
    assert pixel(comp, 640, 44) == BACKGROUND
    print("cancelled logout OK")

    # A broken configuration file: a notice at the top right, closed by its button.
    with open(f"{comp.dir}/config.toml", "a") as f:
        f.write("[bindings]\nnot_a_binding = ['Super+x']\n")
    assert bar.send("theme-reload") == "v1 ok"
    assert wait(lambda: pixel(comp, 1280 - 8 - 390, 76) == ACCENT), pixel(comp, 1280 - 8 - 390, 76)
    click(1280 - 8 - 12 - 12, 46 + 12 + 13)
    assert wait(lambda: pixel(comp, 1280 - 8 - 390, 76) != ACCENT), "the notice did not close"
    print("configuration notice OK")

    # Not cancelled, the countdown ends the session after five seconds.
    comp.key("alt+m")
    assert wait(lambda: comp.process.poll() is not None, 10), "the countdown did not log out"
    assert comp.process.returncode == 0, comp.process.returncode
    assert not os.path.exists(comp.sock)
    print("logout after the countdown OK")
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
