"""Fifo and commit-timing barriers are released in step with the frames, and never stall a hidden window.

Needs `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/pacing_smoke.py
"""
import subprocess, time
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/splice-test-client"


def run(comp, *args):
    return subprocess.Popen([CLIENT, *args], env=comp.env(), stdout=subprocess.PIPE, text=True)


def counts(client, seconds):
    """The client's `fifo <updates> <frames> <applied>` lines for that many seconds."""
    return [tuple(map(int, client.stdout.readline().split()[1:])) for _ in range(seconds)]


def one_per_frame(seconds):
    # Without the barriers the three queued updates would be applied together and share a frame.
    return all(updates > 0 and updates <= frames * 1.1 + 1 for updates, frames, _ in seconds)


comp = Compositor()
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)

    # Shown: one update per frame, although the client queues three ahead. (The nested output has no
    # vblank, so how many frames a second depends on the machine.)
    client = run(comp, "fifo", "20")
    shown = counts(client, 4)[1:]
    print("fifo while shown (updates, frames, applied):", shown)
    assert one_per_frame(shown) and all(u >= 20 for u, _, _ in shown), shown

    # On a hidden workspace (not in the space, no frames at all) the fallback timer releases about once
    # a second: slower, but never stalled.
    assert bar.send("new-workspace 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 2)
    hidden = counts(client, 5)[2:]
    print("fifo while hidden (updates, frames, applied):", hidden)
    assert all(1 <= applied <= 3 for _, _, applied in hidden), hidden

    assert bar.send("workspace 1 1") == "v1 ok"
    bar.until(lambda s: s[0]["active"] == 1)
    back = counts(client, 4)[2:]
    print("fifo shown again (updates, frames, applied):", back)
    assert one_per_frame(back) and all(u >= 20 for u, _, _ in back), back
    client.kill()
    client.wait()
    print("fifo OK")

    # An update meant for 500 ms later is applied then, not at the next frame and not much later. Its
    # release counts on the frame it goes into being at least one refresh interval (17 ms) away; the nested
    # output has no vblank and shows it at once, so here it may come that much early. (The nested output
    # also redraws all the time, so the wakeup for an otherwise idle output needs real hardware.)
    timed = run(comp, "timed", "500")
    line = timed.stdout.readline().split()
    timed.wait(10)
    assert line[0] == "timed", line
    print("timed update applied after", line[1], "ms")
    assert 500 - 17 <= int(line[1]) <= 500 + 200, line
    print("commit timing OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
