"""Workspace overview against the nested compositor: thumbnails and switching by click or keys.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/overview_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor

SURFACE = "#313244"  # default `surface` color: the background of an (empty) thumbnail


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()


comp = Compositor(windows=2)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1 and s[0]["workspaces"].get(1))
    # The focused window moves to a new workspace 2; go back to workspace 1 with the other one.
    comp.key("alt+shift+n")
    bar.until(lambda s: set(s[0]["workspaces"]) == {1, 2} and s[0]["active"] == 1)
    time.sleep(1)
    win = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()

    # Two thumbnails, each with one window.
    comp.key("alt+Tab")
    time.sleep(1)
    shot = f"{comp.dir}/overview.png"
    subprocess.run(["grim", shot], env=comp.env(), check=True, timeout=15)
    assert pixel(shot, 934, 400) != SURFACE, "workspace 2 shows no window"
    assert pixel(shot, 300, 400) != SURFACE, "workspace 1 shows no window"
    assert pixel(shot, 640, 400) != "#1E1E2E", "gap between thumbnails is not dimmed"
    assert pixel(shot, 5, 5) != "#1E1E2E", "the background is not dimmed"

    # Keys: Right selects the second thumbnail, Enter goes there.
    comp.key("Right")
    comp.key("Return")
    bar.until(lambda s: s[0]["active"] == 2)

    # Open again and click the first thumbnail.
    comp.key("alt+Tab")
    time.sleep(0.5)
    for args in (["mousemove", "--window", win, "300", "400"], ["click", "1"]):
        subprocess.run(["xdotool", *args], check=True)
        time.sleep(0.3)
    bar.until(lambda s: s[0]["active"] == 1)

    # Escape closes without switching.
    comp.key("alt+Tab")
    time.sleep(0.3)
    comp.key("Escape")
    time.sleep(0.5)
    subprocess.run(["grim", shot], env=comp.env(), check=True, timeout=15)
    assert pixel(shot, 5, 5) == "#1E1E2E", "the overview is still showing"
    assert bar.state[0]["active"] == 1
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
