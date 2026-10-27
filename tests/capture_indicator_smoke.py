"""The bar learns when something captures, and a shared window gets the capture border.

Needs `cargo build --workspace` and grim. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/capture_indicator_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor, wait

CAPTURE = "./target/debug/splice-capture-test"
# Distinct colors: the capture border must not be the focus border.
CONFIG = "[appearance]\nactive_border = '#0000ff'\ncapture_border = '#ff8800'\nborder_width = 4\n"


def capturing(bar):
    bar.poll()
    return bar.capturing


def border(comp):
    """The pixel on the left border of the one tiled window (gaps 8, border 4)."""
    out = subprocess.run(["grim", "-g", "10,400 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True).stdout
    return out[-3:].hex().upper()


comp = Compositor(config=CONFIG, windows=1)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    time.sleep(2)
    assert not capturing(bar), "capturing before anything captures"
    print("border without capture:", border(comp))

    # A window shared on its own (a stream through the portal): flag and border while the session lives.
    holder = subprocess.Popen([CAPTURE, "toplevel", "hold", "4"], env=comp.env(), stdout=subprocess.PIPE, text=True)
    assert holder.stdout.readline().strip(), "no frame from the window capture"
    assert wait(lambda: capturing(bar), 3), "the bar was not told about the capture"
    shared = border(comp)
    print("border while shared:", shared)
    assert shared == "FF8800", shared
    holder.wait(10)
    # The grim above was a wlr-screencopy frame too; give it its moment to end.
    assert wait(lambda: not capturing(bar), 5), "still capturing after the session ended"
    assert border(comp) == "0000FF"
    print("window session sets flag and border OK")

    # wlr-screencopy has no sessions: one frame counts for a moment, then the flag goes again.
    time.sleep(2)
    subprocess.run(["grim", "-t", "ppm", "/dev/null"], env=comp.env(), check=True)
    assert wait(lambda: capturing(bar), 2), "a screencopy frame did not count"
    assert wait(lambda: not capturing(bar), 4), "the screencopy flag did not end"
    print("screencopy frame sets the flag for a moment OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
