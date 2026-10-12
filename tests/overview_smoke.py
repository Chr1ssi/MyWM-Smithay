"""Workspace overview against the nested compositor: thumbnails, switching by click, keys or digits, `+`, the
scratchpad's cell, dragging windows between cells, closing windows, the urgent frame, every monitor at once and the
zoom when opening and closing.

Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/overview_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor, wait

SURFACE = "#313244"  # default `surface` color: the background of an (empty) cell
MUTED = "#A6ADC8"  # default `muted_text`: the `+`
BACKGROUND = "#1E1E2E"  # the desktop without a wallpaper
# Cells on 1280x800 (see overview_cells). Two workspaces, `+` and the scratchpad: two rows of three.
WS1, WS2, NEW, SCRATCH = (248, 273), (640, 273), (1032, 273), (248, 527)
# One workspace, `+` and the scratchpad: one row.
ONE_WS, ONE_NEW, ONE_SCRATCH = (248, 400), (640, 400), (1032, 400)


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2].upper()


def screenshot(comp, name="overview.png"):
    path = f"{comp.dir}/{name}"
    subprocess.run(["grim", path], env=comp.env(), check=True, timeout=15)
    return path


def nested_window():
    win = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    subprocess.run(["xdotool", "windowfocus", win], check=True)
    return win


def click(at, button=1):
    win = nested_window()
    for args in (["mousemove", "--window", win, str(at[0]), str(at[1])], ["click", str(button)]):
        subprocess.run(["xdotool", *args], check=True)
        time.sleep(0.3)


def hover(at):
    subprocess.run(["xdotool", "mousemove", "--window", nested_window(), str(at[0]), str(at[1])], check=True)
    time.sleep(0.3)


def drag(start, end):
    """Hold the left button on `start` and let go on `end`, moving in steps like a hand would."""
    win = nested_window()
    subprocess.run(["xdotool", "mousemove", "--window", win, str(start[0]), str(start[1]), "mousedown", "1"], check=True)
    time.sleep(0.2)
    for step in range(1, 11):
        x = start[0] + (end[0] - start[0]) * step // 10
        y = start[1] + (end[1] - start[1]) * step // 10
        subprocess.run(["xdotool", "mousemove", "--window", win, str(x), str(y)], check=True)
        time.sleep(0.03)
    subprocess.run(["xdotool", "mouseup", "1"], check=True)
    time.sleep(0.5)


def workspaces(bar):
    return bar.state[0]["workspaces"]


def run(name, test, **compositor):
    comp = Compositor(**compositor)
    try:
        test(comp)
        assert comp.stop()
        print(name, "OK")
    except BaseException:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
        raise


def basics(comp):
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1 and s[0]["workspaces"].get(1))
    # The focused window moves to a new workspace 2; go back to workspace 1 with the other one.
    comp.key("alt+shift+n")
    bar.until(lambda s: set(s[0]["workspaces"]) == {1, 2} and s[0]["active"] == 1)
    time.sleep(1)

    # Two cells with a window each, `+`, the empty scratchpad; the shown workspace is marked.
    comp.key("alt+Tab")
    time.sleep(1)
    shot = screenshot(comp)
    assert pixel(shot, *WS1) != SURFACE, "workspace 1 shows no window"
    assert pixel(shot, *WS2) != SURFACE, "workspace 2 shows no window"
    assert pixel(shot, *NEW) == MUTED, f"no + in its cell: {pixel(shot, *NEW)}"
    assert pixel(shot, *SCRATCH) == SURFACE, "the empty scratchpad's cell is not empty"
    assert pixel(shot, 62, WS1[1]) == "#FFFFFF", f"the selected cell has no white frame: {pixel(shot, 62, WS1[1])}"
    assert pixel(shot, 444, 273) not in (BACKGROUND, SURFACE), "gap between thumbnails is not dimmed"
    assert pixel(shot, 5, 5) != BACKGROUND, "the background is not dimmed"

    # Keys: Right selects the second cell, Enter goes there; a digit goes straight to that workspace.
    comp.key("Right")
    comp.key("Return")
    bar.until(lambda s: s[0]["active"] == 2)
    comp.key("alt+Tab")
    time.sleep(0.5)
    comp.key("1")
    bar.until(lambda s: s[0]["active"] == 1)
    print("keys and digits OK")

    # `+` makes a new workspace and goes there.
    comp.key("alt+Tab")
    time.sleep(0.5)
    click(NEW)
    bar.until(lambda s: set(s[0]["workspaces"]) == {1, 2, 3} and s[0]["active"] == 3)
    comp.key("alt+Tab")
    time.sleep(0.5)
    comp.key("1")
    bar.until(lambda s: set(s[0]["workspaces"]) == {1, 2} and s[0]["active"] == 1)
    print("+ OK")

    # Drag workspace 2's window onto workspace 1: workspace 2 is left empty and goes; the overview stays.
    comp.key("alt+Tab")
    time.sleep(0.5)
    drag(WS2, WS1)
    bar.until(lambda s: set(s[0]["workspaces"]) == {1})
    shot = screenshot(comp)
    assert pixel(shot, 5, 5) != BACKGROUND, "the overview closed after the drop"
    assert pixel(shot, *ONE_NEW) == MUTED, "the overview did not follow the drop"
    # Onto `+`: a new workspace with that window (the left one: two columns meet in the middle of the cell).
    drag((ONE_WS[0] - 90, ONE_WS[1]), ONE_NEW)
    bar.until(lambda s: s[0]["workspaces"] == {1: True, 2: True} and s[0]["active"] == 1)
    # Onto the scratchpad: kept there out of sight, its workspace goes.
    drag(WS2, SCRATCH)
    bar.until(lambda s: set(s[0]["workspaces"]) == {1})
    assert wait(lambda: (bar.poll(), bar.scratchpad == (False, True))[1]), bar.scratchpad
    shot = screenshot(comp)
    assert pixel(shot, *ONE_SCRATCH) != SURFACE, "the scratchpad's cell shows no window"
    print("dragging OK")

    # A click on the scratchpad's cell shows it.
    click((ONE_SCRATCH[0], ONE_SCRATCH[1] + 100))
    assert wait(lambda: (bar.poll(), bar.scratchpad == (True, True))[1]), bar.scratchpad

    # `q` closes the window under the pointer, a middle click the one clicked; the overview stays.
    comp.key("alt+Tab")
    time.sleep(0.5)
    hover(ONE_WS)
    comp.key("q")
    bar.until(lambda s: s[0]["workspaces"] == {1: False})
    click(ONE_SCRATCH, button=2)
    assert wait(lambda: (bar.poll(), bar.scratchpad == (False, False))[1]), bar.scratchpad
    shot = screenshot(comp)
    assert pixel(shot, 5, 5) != BACKGROUND, "closing a window closed the overview"
    print("closing windows OK")

    # Escape closes the overview without switching.
    comp.key("Escape")
    time.sleep(0.5)
    shot = screenshot(comp)
    assert pixel(shot, 5, 5) == BACKGROUND, "the overview is still showing"
    assert bar.state[0]["active"] == 1


def urgent(comp):
    assert wait(lambda: open(comp.dir + "/log").read().count("new window") == 2, 10)
    time.sleep(2)
    comp.key("alt+Tab")
    time.sleep(0.5)
    # Selected, the cell is framed white; selecting another, its urgent window frames it red.
    assert pixel(screenshot(comp), 62, ONE_WS[1]) == "#FFFFFF"
    comp.key("Right")
    time.sleep(0.3)
    frame = pixel(screenshot(comp), 62, ONE_WS[1])
    assert frame == "#F38BA8", f"no urgent frame: {frame}"


def every_monitor(comp):
    """The overview opens on every monitor, not only under the pointer."""
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 2)
    time.sleep(1)

    def surface_pixels(output):
        shot = subprocess.run(["grim", "-o", output, "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True).stdout
        out = subprocess.run(["convert", "ppm:-", "-fill", "black", "+opaque", SURFACE, "-fill", "white", "-opaque", SURFACE,
                              "-format", "%[fx:mean*w*h]", "info:"], input=shot, capture_output=True, check=True).stdout
        return round(float(out))

    before = surface_pixels("virtual-1")
    comp.key("alt+Tab")
    time.sleep(0.5)
    during = surface_pixels("virtual-1")
    print("overview pixels on the monitor without the pointer, before and while open:", before, during)
    assert before < 1000 < 50_000 < during, (before, during)
    comp.key("Escape")


def zoom(comp):
    """With an animation the overview zooms out of the desktop and back into it."""
    bar = Bar(comp.sock)
    bar.until(lambda s: s[0]["workspaces"].get(1))
    time.sleep(1)
    started = time.monotonic()
    comp.key("alt+Tab")
    early = screenshot(comp, "early.png")
    took = time.monotonic() - started
    time.sleep(1.2)
    late = screenshot(comp, "late.png")
    assert pixel(late, 5, 5) != BACKGROUND, "the overview did not open"
    # Eased over 1000 ms: after ~0.4 s it is visibly darker still; a slower machine cannot tell.
    if took < 0.4:
        assert pixel(early, 5, 5) != pixel(late, 5, 5), f"no zoom when opening ({pixel(early, 5, 5)} after {took:.2f} s)"
    else:
        print(f"opening not checked: the first picture took {took:.2f} s")
    # Closing plays it backwards: slow at first, so right after Escape it is mostly there, and then gone.
    comp.key("Escape")
    closing = screenshot(comp, "closing.png")
    assert pixel(closing, 5, 5) != BACKGROUND, "no zoom when closing"
    time.sleep(1.2)
    assert pixel(screenshot(comp, "closed.png"), 5, 5) == BACKGROUND, "the overview did not close"


run("basics", basics, windows=2)
run("urgent", urgent, extra_args="./target/debug/mywm-test-client urgent 40 & sleep 1; ")
run("every monitor", every_monitor, extra_env={"MYWM_VIRTUAL_OUTPUTS": "1"})
run("zoom", zoom, windows=1, config="[effects]\nanimation_ms = 1000\n")
print("OK")
