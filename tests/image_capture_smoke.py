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
    return re.match(r"(\d+)x(\d+) center=([0-9A-F]{6}) alpha=([0-9A-F]{2}) hash=([0-9a-f]+)", out.stdout).groups()


comp = Compositor(windows=1)
try:
    time.sleep(3)
    w, h, center, alpha, _ = capture(comp, "output")
    assert (w, h) == ("1280", "800"), (w, h)
    print("output:", w, h, center)

    w, h, center, alpha, _ = capture(comp, "toplevel")
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


# Further frames of one session redraw only what changed since the one before: they must look like a capture
# drawn from scratch, also where something went away.
comp = Compositor()
try:
    time.sleep(2)
    background = capture(comp, "output")[4]

    def solid(color):
        # A square in the middle of the output, above everything (a layer surface without anchors is centred).
        process = subprocess.Popen(["./target/debug/mywm-test-client", "layer", "overlay", "200", "200", color, "60"],
                                   env=comp.env())
        time.sleep(1)
        return process

    blue = solid("0000ff")
    reference = capture(comp, "output")[4]
    assert reference != background, "the square does not show"
    stream = subprocess.Popen([CLIENT, "output", "frames", "3"], env=comp.env(), stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, text=True)

    def next_frame(change=None):
        if change:
            change()
            time.sleep(0.5)
            stream.stdin.write("\n")
            stream.stdin.flush()
        # The whole picture, not only the middle (the part that changed is drawn anyway).
        return re.match(r"\d+x\d+ center=[0-9A-F]{6} alpha=[0-9A-F]{2} hash=([0-9a-f]+)", stream.stdout.readline()).group(1)

    frame = next_frame()
    assert frame == reference, (frame, reference)
    frame = next_frame(lambda: (blue.kill(), blue.wait()))
    assert frame == background, ("the stream kept a square that went", frame, background)
    red = []
    frame = next_frame(lambda: red.append(solid("ff0000")))
    assert frame == capture(comp, "output")[4] != background, frame
    assert stream.wait(timeout=10) == 0
    for process in red:
        process.kill()
        process.wait()
    print("stream frames follow changes")
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
