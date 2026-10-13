"""Behind the lock only the program bindings marked allow_when_locked run; every other key goes to the locker.

Needs quickshell (the lock screen of the mywm-shell checkout next to this one). Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/locked_bindings_smoke.py
"""
import os, subprocess, tempfile, time
from smoke_support import Bar, Compositor, wait

marks = tempfile.mkdtemp()
CONFIG = f"""
[program_bindings.volume_up]
keys = ["XF86AudioRaiseVolume"]
command = ["touch", "{marks}/volume"]
allow_when_locked = true

[program_bindings.browser]
keys = ["Super+b"]
command = ["touch", "{marks}/browser"]
"""
comp = Compositor(config=CONFIG)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    locker = subprocess.run(["./target/debug/mywm-compositor", "--lock"], env=comp.env(), timeout=15)
    assert locker.returncode == 0
    bar.until(lambda s: bar.locked, 5)
    time.sleep(0.5)
    comp.key("XF86AudioRaiseVolume")
    assert wait(lambda: os.path.exists(f"{marks}/volume"), 5), "the volume key did nothing behind the lock"
    comp.key("alt+b")  # Super+b, nested
    time.sleep(1)
    assert not os.path.exists(f"{marks}/browser"), "a binding without allow_when_locked ran behind the lock"
    print("only allowed bindings run behind the lock OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
