"""Frame callbacks: a window draws at the full rate while it is shown and about once a second while a
fullscreen window covers it.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/frame_throttle_smoke.py
"""
import subprocess
from smoke_support import Compositor

CLIENT = "./target/debug/mywm-test-client"


def rates(process, count):
    """The next `count` per-second frame counts the client prints."""
    return [int(process.stdout.readline().split()[1]) for _ in range(count)]


comp = Compositor()
try:
    env = comp.env()
    shown = subprocess.Popen([CLIENT, "frames", "40"], env=env, stdout=subprocess.PIPE, text=True)
    rates(shown, 1)  # mapping
    visible = rates(shown, 2)
    print("visible:", visible)
    assert min(visible) >= 10, f"a shown window draws too slowly: {visible}"

    cover = subprocess.Popen([CLIENT, "frames", "40", "fullscreen"], env=env, stdout=subprocess.PIPE, text=True)
    rates(cover, 1)
    rates(shown, 1)  # the second in which the fullscreen window appeared
    covered = rates(shown, 3)
    print("covered:", covered)
    assert max(covered) <= 2 and sum(covered) >= 2, f"a covered window should draw about once a second: {covered}"

    cover.kill()
    cover.wait()
    rates(shown, 1)
    uncovered = rates(shown, 2)
    print("uncovered:", uncovered)
    assert min(uncovered) >= 10, f"the window did not get its frames back: {uncovered}"
    shown.kill()
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
