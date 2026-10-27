"""The overview: a click on a window's thumbnail shows that window; picking what to share, it answers the portal
with the window (also one on a hidden workspace) or the monitor.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/overview_share_smoke.py
"""
import socket, subprocess, time
from smoke_support import Bar, Compositor, wait

# Two workspaces on 1280x800, picking what to share: the overview's cells are centered at (346, 400) and (934, 400)
# (see overview_cells). Otherwise `+` and the scratchpad follow, and the first cell is centered at (248, 273).
FIRST_CELL, SECOND_CELL = (346, 400), (934, 400)
FIRST_CELL_WITH_EXTRAS = (248, 273)


def click(at):
    win = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
    for args in (["mousemove", "--window", win, str(at[0]), str(at[1])], ["click", "1"]):
        subprocess.run(["xdotool", *args], check=True)
        time.sleep(0.3)


def ask(comp, kinds):
    """A portal's `choose-source`: the connection that gets the answer."""
    client = socket.socket(socket.AF_UNIX)
    client.connect(comp.sock)
    client.settimeout(10)
    client.sendall(f"v1 choose-source {kinds}\n".encode())
    time.sleep(0.5)
    return client


def answer(client):
    data = b""
    while b"v1 chosen" not in data:
        data += client.recv(4096)
    return next(line for line in data.decode().splitlines() if line.startswith("v1 chosen"))


comp = Compositor(windows=1)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: s[0]["workspaces"].get(1))
    time.sleep(1)
    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2)

    # A click on the window's thumbnail: its workspace is shown again.
    comp.key("alt+Tab")
    time.sleep(0.5)
    click(FIRST_CELL_WITH_EXTRAS)
    bar.until(lambda s: s[0]["active"] == 1)
    print("a thumbnail click shows the window OK")

    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2)
    # Sharing: the window on the hidden workspace, picked from its thumbnail.
    client = ask(comp, "both")
    click(FIRST_CELL)
    chosen = answer(client)
    print("thumbnail:", chosen)
    assert chosen.startswith("v1 chosen window "), chosen
    # The empty cell: the monitor.
    client = ask(comp, "both")
    click(SECOND_CELL)
    assert answer(client) == "v1 chosen monitor winit"
    # Monitors only: a window's thumbnail picks its monitor.
    client = ask(comp, "monitor")
    click(FIRST_CELL)
    assert answer(client) == "v1 chosen monitor winit"
    # Escape: nothing.
    client = ask(comp, "both")
    comp.key("Escape")
    assert answer(client) == "v1 chosen none"
    # A click beside the cells closes the overview, and that answers too: the portal never waits for good.
    client = ask(comp, "both")
    click((20, 20))
    assert answer(client) == "v1 chosen none"
    print("sharing from the overview OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise

# Two monitors: picking what to share, the overview is on both, not only under the pointer.
def surface_pixels(comp, output):
    """Pixels of the overview's empty cells (the `surface` color) on `output`."""
    shot = subprocess.run(["grim", "-o", output, "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True).stdout
    out = subprocess.run(["convert", "ppm:-", "-fill", "black", "+opaque", "#313244", "-fill", "white", "-opaque", "#313244",
                          "-format", "%[fx:mean*w*h]", "info:"], input=shot, capture_output=True, check=True).stdout
    return round(float(out))


comp = Compositor(config='[bindings]\nfocus_output_right = ["Super+Ctrl+period"]\n', extra_env={"SPLICE_VIRTUAL_OUTPUTS": "1"})
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 2)
    time.sleep(1)
    comp.key("alt+ctrl+period")  # the pointer goes to the second monitor
    time.sleep(0.5)
    before = surface_pixels(comp, "winit")
    client = ask(comp, "both")
    during = surface_pixels(comp, "winit")
    print("overview pixels on the monitor without the pointer, before and while picking:", before, during)
    assert before < 1000 < 50_000 < during, (before, during)
    comp.key("Return")
    assert answer(client) == "v1 chosen monitor virtual-1"
    print("the overview is on every monitor; Enter shares the one under the pointer OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
