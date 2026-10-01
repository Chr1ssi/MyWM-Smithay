"""A pointer locked by a window (a game's mouse look) is released when its workspace is no
longer shown, against the nested compositor. A locked pointer produces no motion, so pointer
focus has to follow the layout, not only the mouse.
Needs xdotool. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/pointer_focus_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/mywm-test-client"


def log(comp):
    return open(comp.dir + "/log").read()


def hover(x, y):
    """Move the pointer over the nested window (two steps, so motion arrives)."""
    window = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    for dx in (0, 5):
        subprocess.run(["xdotool", "mousemove", "--window", window, str(x + dx), str(y + dx)], check=True)
        time.sleep(0.2)


# --- A locked pointer is released when another workspace is shown.
comp = Compositor(extra_args=f"{CLIENT} pointer 60 lock & ")
try:
    bar = Bar(comp.sock)
    assert wait(lambda: "pointer client ready" in log(comp), 15), "the client did not start"
    bar.until(lambda s: s[0]["workspaces"].get(1))
    time.sleep(1)
    hover(60, 60)
    assert wait(lambda: "pointer locked" in log(comp), 5), "the pointer was not locked"
    assert bar.send("new-workspace 1") == "v1 ok"
    assert wait(lambda: "pointer unlocked" in log(comp) and "pointer leave" in log(comp), 5), \
        "the hidden window kept the pointer and its lock"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(log(comp)[-3000:])
    raise
