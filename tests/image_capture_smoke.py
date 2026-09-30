"""Window and output capture (ext-image-copy-capture) against the nested compositor.

Needs `cargo build --workspace` (the test client is crates/mywm-capture-test). Run from the
repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/image_capture_smoke.py
"""
import re, subprocess, time
from smoke_support import Compositor

CLIENT = "./target/debug/mywm-capture-test"


def capture(comp, kind):
    out = subprocess.run([CLIENT, kind], env=comp.env(), capture_output=True, text=True, timeout=20)
    assert out.returncode == 0, (kind, out.stdout, out.stderr)
    return re.match(r"(\d+)x(\d+) center=([0-9A-F]{6}) alpha=([0-9A-F]{2})", out.stdout).groups()


comp = Compositor(windows=1)
try:
    time.sleep(3)
    w, h, center, alpha = capture(comp, "output")
    assert (w, h) == ("1280", "800"), (w, h)
    print("output:", w, h, center)

    w, h, center, alpha = capture(comp, "toplevel")
    # One tiled window fills the work area minus gaps: smaller than the output, and opaque.
    assert 0 < int(w) < 1280 and 0 < int(h) < 800, (w, h)
    assert alpha == "FF", alpha
    assert center != "1E1E2E", "captured the compositor background, not the window"
    print("toplevel:", w, h, center)
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
