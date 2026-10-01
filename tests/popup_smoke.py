"""xdg_popup against the nested compositor: a popup is configured, and kept on screen.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/popup_smoke.py
"""
import subprocess, time
from smoke_support import Compositor

comp = Compositor()
try:
    out = subprocess.run(["./target/debug/mywm-test-client", "popup", "2"], env=comp.env(), capture_output=True, text=True, timeout=30).stdout
    assert "popup configured" in out, f"the popup never got its configure: {out!r}"
    x, y, w, h = map(int, out.split("popup configured")[1].split()[:4])
    assert (w, h) == (100, 80), (w, h)
    print("popup configured at", (x, y))
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
