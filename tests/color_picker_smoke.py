"""The color picker: the bar learns the color under the pointer, a magnifier shows it next to the pointer, a click
copies it as #rrggbb, Escape and other buttons cancel.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/color_picker_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor, wait

ORANGE = "#ff8800"


def nested_window():
    return subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()


def x_pixel(x, y):
    """A pixel of the nested window as X shows it: with cursor and overlays, unlike a capture."""
    shot = subprocess.check_output(["import", "-window", nested_window(), "png:-"])
    out = subprocess.run(["convert", "png:-", "-crop", f"1x1+{x}+{y}", "txt:-"], input=shot, capture_output=True,
                         check=True).stdout.decode()
    return out.strip().splitlines()[-1].split()[2].upper()


def clipboard(comp):
    result = subprocess.run(["wl-paste", "--no-newline"], env=comp.env(), capture_output=True, text=True, timeout=5)
    return result.stdout


# swaybg fills the background with one color: what the picker has to find.
comp = Compositor(config='[bindings]\ncolor_picker = ["Super+Shift+c"]\n',
                  extra_args=f"swaybg -c '{ORANGE}' & sleep 1; ")
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: s[0]["workspaces"].get(1) is not None)
    time.sleep(1.5)
    win = nested_window()
    subprocess.run(["xdotool", "mousemove", "--window", win, "400", "300"], check=True)
    time.sleep(0.3)

    comp.key("alt+shift+c")
    assert wait(lambda: (bar.poll(), bar.picking == ORANGE)[1]), bar.picking
    time.sleep(0.5)
    # The magnifier starts 24 px below right of the pointer: 11 pixels of 12 px with a white border.
    assert x_pixel(400 + 24 + 30, 300 + 24 + 30) == ORANGE.upper(), x_pixel(454, 354)
    assert x_pixel(400 + 24 - 1, 300 + 24 + 30) == "#FFFFFF", "no border around the magnifier"
    print("magnifier OK")

    # The picture changes while picking: the sample follows it both ways (each sample redraws only what changed).
    blue = subprocess.Popen(["./target/debug/splice-test-client", "fullscreen", "0000ff", "30"], env=comp.env())
    assert wait(lambda: (bar.poll(), bar.picking == "#0000ff")[1]), bar.picking
    blue.kill()
    blue.wait()
    assert wait(lambda: (bar.poll(), bar.picking == ORANGE)[1]), bar.picking
    print("sample follows changes OK")

    subprocess.run(["xdotool", "click", "1"], check=True)
    assert wait(lambda: (bar.poll(), bar.picking is None)[1]), "picking did not end with the click"
    assert wait(lambda: clipboard(comp) == ORANGE), clipboard(comp)
    print("click copies the color OK")

    # Escape and a right click end it without copying.
    subprocess.run(["wl-copy", "unverändert"], env=comp.env(), check=True)
    for cancel in (lambda: comp.key("Escape"), lambda: subprocess.run(["xdotool", "click", "3"], check=True)):
        comp.key("alt+shift+c")
        assert wait(lambda: (bar.poll(), bar.picking is not None)[1])
        cancel()
        assert wait(lambda: (bar.poll(), bar.picking is None)[1]), "picking did not end"
    time.sleep(0.5)
    assert clipboard(comp) == "unverändert", clipboard(comp)
    assert x_pixel(454, 354) == ORANGE.upper(), "the magnifier stayed"
    print("cancel OK")
    assert comp.stop()
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
