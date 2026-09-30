"""Tiles scrolled off one monitor's edge must not show up on its neighbour.

Three windows on the first monitor leave the third one beyond the right edge, where the second
monitor (a virtual output here) starts in the shared coordinate space. Captures the neighbour
with grim and checks that it only shows the background.
Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/scroll_clip_smoke.py
"""
import subprocess, time
from smoke_support import Compositor


def colors(path):
    out = subprocess.check_output(["convert", path, "-unique-colors", "txt:-"]).decode()
    return len(out.strip().splitlines()) - 1


comp = Compositor(windows=3, extra_env={"MYWM_VIRTUAL_OUTPUTS": "1"})
try:
    time.sleep(4)
    # Focus the first column so the row scrolls back and the last window hangs over the right edge.
    comp.key("alt+h")
    comp.key("alt+h")
    time.sleep(1)
    env = comp.env()
    main, neighbour = f"{comp.dir}/main.png", f"{comp.dir}/neighbour.png"
    subprocess.run(["grim", "-o", "winit", main], env=env, check=True, timeout=15)
    subprocess.run(["grim", "-o", "virtual-1", neighbour], env=env, check=True, timeout=15)
    assert colors(main) > 50, "the first monitor should show the windows"
    n = colors(neighbour)
    assert n == 1, f"the neighbouring monitor shows {n} colors: an off-screen tile leaked over"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
