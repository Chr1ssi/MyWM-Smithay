"""xdg-foreign: a dialog that imports another client's window as its parent (a portal's file chooser) is a
dialog of that window, floating like other dialogs, whether the parent comes before its first commit or later.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/foreign_smoke.py
"""
import subprocess
from smoke_support import Compositor, wait

CLIENT = "./target/debug/splice-test-client"


def dialog_sizes(comp, *args):
    """The dialog's sizes: tiled it gets half the screen beside its parent, floating (0, 0), its own choice."""
    client = subprocess.Popen([CLIENT, "foreign", "6", *args], env=comp.env(), stdout=subprocess.PIPE, text=True)
    lines = client.stdout.read().split("\n")
    client.wait(10)
    return lines


comp = Compositor()
try:
    early = dialog_sizes(comp)
    print("parent before the first commit:", early)
    sizes = [tuple(map(int, line.split()[1:])) for line in early if line.startswith("size")]
    assert sizes and all(w < 600 for w, _ in sizes), sizes

    late = dialog_sizes(comp, "late")
    print("parent after placing:", late)
    index = late.index("parented")
    before = [tuple(map(int, l.split()[1:])) for l in late[:index] if l.startswith("size")]
    after = [tuple(map(int, l.split()[1:])) for l in late[index:] if l.startswith("size")]
    # Tiled first (beside its parent), floating once it is a dialog.
    assert before and before[-1][0] >= 600, before
    assert after and after[-1][0] < 600, after
    print("xdg-foreign dialogs float OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
