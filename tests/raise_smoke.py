"""Run or raise (a program binding with app_id) and focus_urgent show the window wherever it is.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/raise_smoke.py
"""
import os, subprocess, tempfile, time
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/splice-test-client"
marker = tempfile.mktemp()
CONFIG = f"""
[bindings]
focus_urgent = ["Super+u"]

[program_bindings.notes]
keys = ["Super+o"]
command = ["touch", "{marker}"]
app_id = "splice.test.n*"
"""


def windows(comp):
    return open(comp.dir + "/log").read().count("new window")


comp = Compositor(config=CONFIG)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    # No window of the app: the command starts it.
    comp.key("alt+o")  # Super+o, nested
    assert wait(lambda: os.path.exists(marker), 5), "run or raise did not run the command"
    os.remove(marker)
    print("runs the command without a window OK")

    subprocess.Popen([CLIENT, "named", "30", "splice.test.notes", "-"], env=comp.env(), stdout=subprocess.DEVNULL)
    assert wait(lambda: windows(comp) == 1, 10)
    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2)
    comp.key("alt+o")
    bar.until(lambda s: s[0]["active"] == 1)
    time.sleep(0.5)
    assert not os.path.exists(marker), "started another instance although a window is open"
    print("raises the open window on another workspace OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# Two windows, the second asks to activate the first without user input: the first is urgent.
comp = Compositor(config=CONFIG, extra_args=f"{CLIENT} urgent 40 & sleep 1; ")
try:
    bar = Bar(comp.sock)
    assert wait(lambda: windows(comp) == 2, 10)
    time.sleep(2)
    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2)
    comp.key("alt+u")
    bar.until(lambda s: s[0]["active"] == 1)
    print("focus_urgent shows the urgent window OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
