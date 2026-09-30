"""A layer surface with exclusive keyboard interactivity (launcher, wallpaper picker) gets the
keyboard as soon as it is shown, without a click.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/layer_focus_smoke.py
"""
import subprocess, time, select
from smoke_support import Compositor, wait

CLIENT = "./target/debug/mywm-test-client"

comp = Compositor(windows=1)
try:
    time.sleep(3)
    client = subprocess.Popen([CLIENT, "layer", "overlay", "420", "300", "0000FF", "20", "exclusive"], env=comp.env(),
                              stdout=subprocess.PIPE, text=True)
    got = []
    end = time.time() + 8
    while time.time() < end and "keyboard focus" not in got:
        ready, _, _ = select.select([client.stdout], [], [], 0.2)
        if ready:
            got.append(client.stdout.readline().strip())
    client.kill()
    assert "keyboard focus" in got, f"the launcher-like layer never got the keyboard: {got}"
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-2500:])
    raise
