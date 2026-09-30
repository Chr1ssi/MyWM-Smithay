"""Copy and paste between clients (wl-clipboard) against the nested compositor.

Two paths are covered: the ordinary one, where a client gets the selection because it has
keyboard focus (MYWM_NO_DATA_CONTROL hides wlr-data-control so wl-copy/wl-paste fall back to
it), and the data-control one used by clipboard managers.
Needs wl-clipboard. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/clipboard_smoke.py
"""
import subprocess, time
from smoke_support import Compositor


def roundtrip(comp, primary=False):
    env = comp.env()
    flags = ["--primary"] if primary else []
    subprocess.run(["wl-copy", *flags, "hello from a test"], env=env, check=True, timeout=15)
    pasted = subprocess.run(["wl-paste", "--no-newline", *flags], env=env, capture_output=True, timeout=15)
    assert pasted.stdout == b"hello from a test", (pasted.stdout, pasted.stderr)


for name, extra in (("focus-based", {"MYWM_NO_DATA_CONTROL": "1"}), ("data-control", {})):
    comp = Compositor(extra_env=extra)
    try:
        time.sleep(2)
        roundtrip(comp)
        roundtrip(comp, primary=True)
        assert comp.stop()
        print(f"{name} clipboard OK")
    except BaseException:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
        raise
print("OK")
