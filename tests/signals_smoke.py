"""Programs the compositor starts can be stopped with SIGTERM and SIGINT: they must not inherit a
blocked signal mask (an event loop that reads signals through signalfd would leave one behind).

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/signals_smoke.py
"""
import os, subprocess
from smoke_support import Compositor, wait

SIGINT, SIGTERM = 2, 15

out = None
comp = None
try:
    marker = os.path.join(os.environ.get("TMPDIR", "/tmp"), f"mywm-signals-{os.getpid()}")
    # The startup command runs through a shell; its child reports the mask it was given.
    comp = Compositor(extra_args=f"grep SigBlk /proc/self/status > {marker}; ")
    assert wait(lambda: os.path.exists(marker) and open(marker).read().strip(), 10), "the startup command did not run"
    blocked = int(open(marker).read().split()[1], 16)
    os.remove(marker)
    for signal, name in ((SIGINT, "SIGINT"), (SIGTERM, "SIGTERM")):
        assert not blocked & (1 << (signal - 1)), f"{name} is blocked in started programs (SigBlk {blocked:#x})"
    # The compositor itself still ends cleanly on SIGTERM.
    assert comp.stop(), "socket not removed on SIGTERM"
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-2000:])
    raise
