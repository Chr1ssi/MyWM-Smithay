"""A window a rule keeps out of captures is black in every capture, and only there.

Needs `cargo build --workspace`, grim and ImageMagick. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/capture_block_smoke.py
"""
import glob, re, subprocess, tempfile, time
from smoke_support import Compositor, wait

CAPTURE = "./target/debug/splice-capture-test"
BLACK = "000000"


def capture(comp, kind):
    """Center of an ext-image-copy-capture frame of the output or the window."""
    out = subprocess.run([CAPTURE, kind], env=comp.env(), capture_output=True, text=True, timeout=20)
    assert out.returncode == 0, (kind, out.stdout, out.stderr)
    return re.match(r"\d+x\d+ center=([0-9A-F]{6})", out.stdout).group(1)


def grim(comp):
    """The window's center through wlr-screencopy."""
    ppm = subprocess.check_output(["grim", "-g", "640,400 1x1", "-t", "ppm", "-"], env=comp.env())
    return ppm[-3:].hex().upper()


def pixel(path):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+640+400", "-depth", "8", "rgb:-"])
    return out.hex().upper()


def screen_file():
    """What the nested window shows on the X server: the screen, not a capture."""
    path = tempfile.mktemp(suffix=".png")
    subprocess.run(["import", "-window", "root", "-crop", "1280x800+0+0", path], check=True)
    return path


def screen():
    return pixel(screen_file())


def black_pixels(path):
    out = subprocess.check_output(["convert", path, "-fill", "white", "+opaque", "black", "-format", "%[fx:mean*w*h]", "info:"])
    return round(1280 * 800 - float(out))


shots = tempfile.mkdtemp()
rule = "[[rules]]\napp_id = 'splice.test.*'\nblock_from_capture = true\n"
comp = Compositor(config=rule, windows=1, top=f'screenshot_directory = "{shots}"\n')
try:
    time.sleep(3)
    results = {"output": capture(comp, "output"), "toplevel": capture(comp, "toplevel"), "screencopy": grim(comp)}
    comp.key("alt+shift+s")
    assert wait(lambda: glob.glob(shots + "/*.png"), 10), "no screenshot written"
    time.sleep(0.5)
    results["screenshot"] = pixel(glob.glob(shots + "/*.png")[0])
    print("captures:", results)
    assert all(center == BLACK for center in results.values()), results
    shown = screen()
    print("screen:", shown)
    assert shown != BLACK, "the window is black on the screen too"

    # The overview shows the window as a thumbnail elsewhere: black in a capture, not on the screen.
    comp.key("alt+Tab")
    time.sleep(1)
    captured = tempfile.mktemp(suffix=".png")
    subprocess.run(["grim", captured], env=comp.env(), check=True)
    in_capture, on_screen = black_pixels(captured), black_pixels(screen_file())
    print("black pixels in the overview, capture and screen:", in_capture, on_screen)
    assert in_capture > 10_000 and on_screen < in_capture // 10, (in_capture, on_screen)
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("blocked window black in every capture, shown on screen OK")

comp = Compositor(windows=1)
try:
    time.sleep(3)
    plain = {"output": capture(comp, "output"), "screencopy": grim(comp)}
    print("without the rule:", plain)
    assert all(center != BLACK for center in plain.values()), plain
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
