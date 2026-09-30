"""Smoke test of the bar socket against the nested compositor.

Run from the repository root with $DISPLAY set (e.g. Xvfb :99), after `cargo build`:
    PYTHONPATH=tests python3 tests/ipc_smoke.py
"""
import os, socket, stat, time
from smoke_support import Bar, Compositor, wait

comp = Compositor(windows=3)
try:
    mode = stat.S_IMODE(os.stat(comp.sock).st_mode)
    assert mode == 0o600, oct(mode)
    bar = Bar(comp.sock)
    bar.until(lambda s: s[0]["workspaces"].get(1))
    time.sleep(3)
    bar.poll()
    print("initial:", bar.state)
    assert bar.state[0]["left"], "three columns on one screen scroll: the first is cut off on the left"

    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2 and set(s[0]["workspaces"]) == {1, 2})
    assert bar.send("workspace 1 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 1 and set(s[0]["workspaces"]) == {1})
    for bad in ["workspace 9 1", "workspace 1 7", "frobnicate"]:
        assert bar.send(bad) == "v1 error invalid-command", bad
    assert bar.send("scratchpad") == "v1 ok"  # empty scratchpad: valid, no change

    # A one-shot sender that closes right after writing still gets its command carried out.
    one_shot = socket.socket(socket.AF_UNIX)
    one_shot.connect(comp.sock)
    one_shot.sendall(b"v1 new-workspace 1\n")
    one_shot.close()
    bar.until(lambda s: s[0]["active"] == 2)

    assert comp.stop(), "socket not removed on exit"
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
