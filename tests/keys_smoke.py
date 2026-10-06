"""Keys the compositor keeps for a binding stay away from clients, their release included.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/keys_smoke.py
"""
import subprocess, time
from smoke_support import Compositor, wait

CLIENT = "./target/debug/mywm-test-client"
KEY_A, KEY_SPACE = 30, 57

# The launcher binding (Super+Space, Alt+Space while nested) runs a program that opens nothing, so the
# window keeps the keyboard focus, as it does before a real launcher has appeared.
comp = Compositor(top='launcher = ["true"]\n')
try:
    out = open(comp.dir + "/client", "w")
    subprocess.Popen([CLIENT, "keys", "30"], env=comp.env(), stdout=out)
    log = lambda: open(comp.dir + "/client").read()
    assert wait(lambda: "keys client ready" in log(), 10), "the keys client did not start"
    time.sleep(1)
    comp.key("alt+space")
    comp.key("a")
    assert wait(lambda: f"key {KEY_A} released" in log(), 5), "the window gets no keys: " + log()
    keys = log()
    assert f"key {KEY_SPACE}" not in keys, "the launcher binding's Space reached the window:\n" + keys
    assert comp.stop()
    print("binding keys stay with the compositor OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
