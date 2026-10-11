"""The exit binding asks the bar that confirms logouts, and ends the session at once without one.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/logout_smoke.py
"""
import time
from smoke_support import Bar, Compositor, wait

# Nested, Super is Alt: the default exit binding Super+m is Alt+m here.
comp = Compositor()
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    assert bar.send("handle-logout") == "v1 ok"
    comp.key("alt+m")
    assert wait(lambda: (bar.poll(), bar.confirm_logouts)[1]), "the bar was not asked to confirm"
    assert bar.confirm_logouts == [bar.state[0]["id"]], bar.confirm_logouts
    time.sleep(0.5)
    assert comp.process.poll() is None, "the exit binding ended the session although a bar confirms logouts"
    print("exit binding asks the bar OK")
    # The confirmed logout ends the session instead of asking again.
    bar.socket.sendall(b"v1 logout\n")
    assert wait(lambda: comp.process.poll() is not None), "logout did not end the session"
    print("logout ends the session OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# The bar that registered went away: the binding must not leave the user stuck.
comp = Compositor()
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    assert bar.send("handle-logout") == "v1 ok"
    bar.socket.close()
    time.sleep(0.5)
    comp.key("alt+m")
    assert wait(lambda: comp.process.poll() is not None), "the exit binding did nothing without a bar"
    print("exit binding without a bar ends the session OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
