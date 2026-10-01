"""Copy and paste between Wayland and X11 clients (Xwayland), both directions.

Needs wl-clipboard, xclip, Xwayland and xterm. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/xclipboard_smoke.py
"""
import re, subprocess, time
from smoke_support import Compositor, wait

comp = Compositor(extra_args="xterm -e 'sleep 60' & ")
try:
    log = lambda: open(comp.dir + "/log").read()
    assert wait(lambda: 'app_id=Some("XTerm")' in log(), 15), "no X11 window"
    xdisplay = re.search(r"Xwayland on DISPLAY=(:\d+)", log()).group(1)
    xenv = dict(comp.env(), DISPLAY=xdisplay)
    time.sleep(1)

    for selection, wl_flags in (("clipboard", []), ("primary", ["--primary"])):
        # Wayland -> X11
        subprocess.run(["wl-copy", *wl_flags, f"from wayland {selection}"], env=comp.env(), check=True, timeout=15)
        time.sleep(0.5)
        out = subprocess.run(["xclip", "-o", "-t", "UTF8_STRING", "-selection", selection], env=xenv, capture_output=True, text=True, timeout=15)
        assert out.stdout == f"from wayland {selection}", (selection, "wayland->x11", out.stdout, out.stderr)
        # X11 -> Wayland
        owner = subprocess.Popen(["xclip", "-i", "-t", "UTF8_STRING", "-selection", selection, "-loops", "3"], env=xenv, stdin=subprocess.PIPE, text=True)
        owner.stdin.write(f"from x11 {selection}")
        owner.stdin.close()
        time.sleep(0.7)
        out = subprocess.run(["wl-paste", "--no-newline", *wl_flags], env=comp.env(), capture_output=True, text=True, timeout=15)
        assert out.stdout == f"from x11 {selection}", (selection, "x11->wayland", out.stdout, out.stderr)
        owner.kill()
        print(selection, "both ways OK")
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(log()[-3000:])
    raise
