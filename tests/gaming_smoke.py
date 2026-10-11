"""A fullscreen game keeps the session from going idle and tells the bar; other fullscreen windows do not.

The game is recognized by its `game` content type alone (its app id matches no game prefix).
Needs swayidle and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/gaming_smoke.py
"""
import os, subprocess, time
from smoke_support import Bar, Compositor, wait

CLIENT = "./target/debug/mywm-test-client"
idle_dir = os.path.join(os.environ.get("TMPDIR", "/tmp"), f"gaming-idle-{os.getpid()}")
os.makedirs(idle_dir, exist_ok=True)
IDLE = f"{idle_dir}/idle"
# After 2 s without input swayidle writes `idle`.
comp = Compositor(extra_args=f"swayidle -w timeout 2 'touch {IDLE}' & ")


def stays_awake(seconds):
    """Input first: swayidle reports idleness only once until something happens."""
    subprocess.run(["xdotool", "mousemove", "300", "300"], check=True)
    subprocess.run(["xdotool", "mousemove", "310", "310"], check=True)
    time.sleep(0.3)
    if os.path.exists(IDLE):
        os.remove(IDLE)
    return not wait(lambda: os.path.exists(IDLE), seconds)


try:
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    assert wait(lambda: os.path.exists(IDLE), 8), "swayidle never saw the session go idle"

    # A fullscreen window that is no game: the session still goes idle.
    other = subprocess.Popen([CLIENT, "frames", "30", "fullscreen"], env=comp.env(), stdout=subprocess.DEVNULL)
    bar.until(lambda s: s[0]["workspaces"].get(1))
    time.sleep(1)
    bar.poll()
    assert not bar.gaming, "a fullscreen window without the game content type counts as a game"
    assert not stays_awake(5), "a fullscreen window that is no game kept the session awake"
    other.kill()
    other.wait()
    print("fullscreen non-game OK")

    game = subprocess.Popen([CLIENT, "frames", "30", "fullscreen", "game"], env=comp.env(), stdout=subprocess.DEVNULL)
    bar.until(lambda s: 0 in s[0]["workspaces"] and s[0]["active"] == 0)
    assert wait(lambda: (bar.poll(), bar.gaming)[1]), "the bar was not told about the game"
    print("game content type puts the window on the gaming workspace OK")
    assert stays_awake(5), "the session went idle under a fullscreen game"
    print("no idle under a fullscreen game OK")

    game.kill()
    game.wait()
    assert wait(lambda: (bar.poll(), not bar.gaming)[1]), "gaming stayed on after the game exited"
    assert not stays_awake(5), "the session stays awake after the game exited"
    print("idle again after the game OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
print("OK")
