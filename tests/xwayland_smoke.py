"""X11 clients through Xwayland, and switching Xwayland off.

Needs Xwayland, xterm and xeyes. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/xwayland_smoke.py
"""
import time
from smoke_support import Bar, Compositor, wait


def log(comp):
    return open(comp.dir + "/log").read()


comp = Compositor(extra_args="xterm -e 'sleep 60' & xeyes & ")
try:
    bar = Bar(comp.sock)
    assert wait(lambda: 'app_id=Some("XTerm")' in log(comp) and 'app_id=Some("XEyes")' in log(comp), 15), \
        "X11 windows were not managed"
    bar.until(lambda s: s[0]["workspaces"].get(1))
    assert "X11 window manager ready" in log(comp)
    # Two X11 windows tile like any others: three would scroll, two fit exactly.
    time.sleep(1)
    bar.poll()
    assert not bar.state[0]["left"] and not bar.state[0]["right"], bar.state
    assert comp.stop()
    print("Xwayland clients OK")
except BaseException:
    comp.process.kill()
    print(log(comp)[-3000:])
    raise

off = Compositor(top="xwayland = false\n", extra_args="sleep 2 & ")
try:
    time.sleep(3)
    assert "Xwayland on" not in log(off), "xwayland = false was ignored"
    assert off.stop()
    print("xwayland = false OK")
    print("OK")
except BaseException:
    off.process.kill()
    print(log(off)[-3000:])
    raise
