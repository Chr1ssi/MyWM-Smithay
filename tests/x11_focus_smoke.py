"""Keyboard input reaches a focused X11 window (Xwayland), and the X11 input focus follows.

Wine/Proton games only get keys once the window manager has set the X11 input focus.
Needs Xwayland, xev and xdotool. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/x11_focus_smoke.py
"""
import os, time
from smoke_support import Compositor, wait

events = os.path.join(os.environ.get("TMPDIR", "/tmp"), f"xev-{os.getpid()}.txt")
comp = Compositor(extra_args=f"xev -event keyboard > {events} 2>&1 & ")
try:
    log = lambda: open(comp.dir + "/log").read()
    assert wait(lambda: "new window" in log(), 15), "xev was not managed"
    assert wait(lambda: "X11 input focus" in log(), 5), "the X11 input focus was never set"
    time.sleep(1)
    comp.key("a")
    time.sleep(0.5)
    comp.key("b")
    assert wait(lambda: os.path.exists(events) and "keysym 0x62" in open(events).read(), 5), \
        "keys never arrived: " + (open(events).read()[-500:] if os.path.exists(events) else "no output")
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(log()[-2500:])
    raise
