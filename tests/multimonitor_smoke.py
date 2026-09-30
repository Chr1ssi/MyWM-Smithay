"""Multi-monitor logic against the nested compositor with virtual outputs.

Run from the repository root with $DISPLAY set: python3 tests/multimonitor_smoke.py
"""
import time
from smoke_support import Bar, Compositor, wait

# Nested, Super is mapped to Alt; bindings that already use Alt are unreachable, so move them.
CONFIG = """
[bindings]
focus_output_left = ["Super+Ctrl+comma"]
focus_output_right = ["Super+Ctrl+period"]
"""
comp = Compositor(config=CONFIG, windows=2, extra_env={"MYWM_VIRTUAL_OUTPUTS": "2"})
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 3 and s[0]["workspaces"].get(1))
    s = bar.state
    print("outputs:", [(o["id"], o["x"], o["width"], sorted(o["workspaces"])) for o in s])
    assert [o["id"] for o in s] == [1, 2, 3]
    assert [o["x"] for o in s] == [0, s[0]["width"], s[0]["width"] + 1280]
    assert [list(o["workspaces"]) for o in s] == [[1], [2], [3]], "one home workspace per monitor"
    time.sleep(2)

    # Move the focused window to the monitor on the right (end of the columns).
    # The newest window is focused and last in its row, so it goes to the next monitor.
    comp.key("alt+shift+Right")
    bar.until(lambda s: s[1]["workspaces"][2] and s[0]["workspaces"][1])
    print("after move right:", [o["workspaces"] for o in bar.state])

    # Focus moved along with the window, so this goes one monitor further right; the new
    # workspace is created there and takes the lowest free number.
    comp.key("alt+ctrl+period")
    comp.key("alt+n")
    bar.until(lambda s: set(s[2]["workspaces"]) == {3, 4} and s[2]["active"] == 4)
    print("after new workspace:", [o["workspaces"] for o in bar.state])

    # Leaving an empty extra removes it; numbers select the owning monitor's workspace.
    comp.key("alt+3")
    bar.until(lambda s: s[2]["active"] == 3 and set(s[2]["workspaces"]) == {3})

    # Bar commands: validated per output.
    assert bar.send("new-workspace 2") == "v1 ok"
    bar.until(lambda s: s[1]["active"] == 3 or s[1]["active"] == 4)
    assert bar.send("workspace 2 2") == "v1 ok"
    bar.until(lambda s: s[1]["active"] == 2 and set(s[1]["workspaces"]) == {2})
    assert bar.send("workspace 1 2") == "v1 error invalid-command", "workspace 2 does not live on output 1"
    assert bar.send("workspace 9 1") == "v1 error invalid-command"
    assert bar.send("workspace 1 7") == "v1 error invalid-command"

    # Windows follow their workspace across monitors: focus the monitor with the window on it.
    comp.key("alt+ctrl+comma")
    comp.key("alt+shift+9")  # workspace 9 does not exist: the window stays where it is
    bar.poll()
    assert bar.state[1]["workspaces"][2]
    print("final:", [(o["active"], o["workspaces"]) for o in bar.state])
    assert comp.stop(), "socket not removed on exit"
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
