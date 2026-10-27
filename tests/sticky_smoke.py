"""Sticky windows (rule or toggle_sticky) come along onto whatever workspace their monitor shows.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/sticky_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/splice-test-client"
CONFIG = """
[bindings]
toggle_sticky = ["Super+p"]

[[rules]]
app_id = "splice.test.pip"
sticky = true
"""


def windows(comp):
    return open(comp.dir + "/log").read().count("new window")


def named(comp, app_id):
    subprocess.Popen([CLIENT, "named", "60", app_id, "-"], env=comp.env(), stdout=subprocess.DEVNULL)


def occupied(state):
    return {n for n, full in state[0]["workspaces"].items() if full}


comp = Compositor(config=CONFIG)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    named(comp, "splice.test.notes")
    assert wait(lambda: windows(comp) == 1, 10)
    named(comp, "splice.test.pip")
    assert wait(lambda: windows(comp) == 2, 10)
    time.sleep(1)

    # The rule's window comes along; the ordinary one stays.
    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2 and occupied(s) == {1, 2})
    print("a sticky window (rule) comes along onto a new workspace OK")
    # Back: the sticky window leaves the extra workspace, which is empty now and goes.
    assert bar.send("workspace 1 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 1 and set(s[0]["workspaces"]) == {1})
    print("the empty extra workspace it left goes away OK")

    # toggle_sticky on the focused ordinary window: it floats and comes along too.
    assert bar.send("workspace 1 1") == "v1 ok"
    time.sleep(0.3)
    comp.key("alt+h")  # focus the left column: the ordinary window
    comp.key("alt+p")  # Super+p, nested: toggle_sticky
    time.sleep(0.3)
    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2 and occupied(s) == {2})
    print("toggle_sticky brings the focused window along OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
