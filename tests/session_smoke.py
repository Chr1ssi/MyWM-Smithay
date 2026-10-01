"""Screen lock, idle notification and monitor power against the nested compositor.

Needs swaylock, swayidle and wlopm besides the usual tools.
Run from the repository root with $DISPLAY set: PYTHONPATH=tests python3 tests/session_smoke.py
"""
import os, subprocess, time
from smoke_support import Bar, Compositor, wait


def pixel(path, x, y):
    out = subprocess.check_output(["convert", path, "-crop", "1x1+%d+%d" % (x, y), "txt:-"]).decode()
    return out.strip().splitlines()[-1].split()[2]  # e.g. #1E1E2E


def screenshot(path):
    subprocess.run(["import", "-window", "root", path], check=True)


idle_dir = os.path.join(os.environ.get("TMPDIR", "/tmp"), f"idle-{os.getpid()}")
os.makedirs(idle_dir, exist_ok=True)
# swayidle runs as a client of the compositor: after 2 s without input it writes `idle`,
# on activity it writes `resume`.
idle_client = f"swayidle -w timeout 2 'touch {idle_dir}/idle' resume 'touch {idle_dir}/resume' & "
comp = Compositor(windows=2, extra_args=idle_client)
try:
    bar = Bar(comp.sock)
    bar.until(lambda s: s[0]["workspaces"].get(1))
    time.sleep(3)

    # --- idle notification (ext-idle-notify) ---
    assert wait(lambda: os.path.exists(f"{idle_dir}/idle"), 8), "swayidle never saw the session go idle"
    subprocess.run(["xdotool", "mousemove", "300", "300"], check=True)
    subprocess.run(["xdotool", "mousemove", "310", "310"], check=True)
    assert wait(lambda: os.path.exists(f"{idle_dir}/resume"), 5), "input did not end idleness"
    print("idle notification OK")

    # --- idle inhibition (idle-inhibit) ---
    env = comp.env()
    # The client replaces its first inhibitor with a second one on the same surface and then exits
    # without destroying the second.
    client = subprocess.Popen(["./target/debug/mywm-test-client", "idle-inhibit", "6"], env=env,
                              stdout=subprocess.PIPE, text=True)
    assert client.stdout.readline().strip() == "inhibiting", "the idle-inhibit client did not start"
    if os.path.exists(f"{idle_dir}/idle"):
        os.remove(f"{idle_dir}/idle")
    assert not wait(lambda: os.path.exists(f"{idle_dir}/idle"), 4), "went idle despite a live inhibitor"
    client.wait(10)
    assert wait(lambda: os.path.exists(f"{idle_dir}/idle"), 6), "still inhibited after the client exited"
    print("idle inhibition OK")

    # --- monitor power (wlr-output-power-management) ---
    listing = subprocess.check_output(["wlopm"], env=env).decode()
    assert "winit on" in listing, listing
    subprocess.run(["wlopm", "--off", "winit"], env=env, check=True)
    assert wait(lambda: "winit off" in subprocess.check_output(["wlopm"], env=env).decode())
    subprocess.run(["wlopm", "--on", "winit"], env=env, check=True)
    assert wait(lambda: "winit on" in subprocess.check_output(["wlopm"], env=env).decode())
    print("output power OK")

    # --- session lock (ext-session-lock via swaylock) ---
    screenshot(f"{comp.dir}/before.png")
    before = pixel(f"{comp.dir}/before.png", 300, 400)
    result = subprocess.run(["./target/debug/mywm-compositor", "--lock"], env=env, timeout=15)
    assert result.returncode == 0, "--lock did not get a confirmation"
    bar.until(lambda s: bar.locked, 5)
    time.sleep(1)
    screenshot(f"{comp.dir}/locked.png")
    locked = pixel(f"{comp.dir}/locked.png", 300, 400)
    print("pixel before lock:", before, "while locked:", locked)
    assert before != locked, "windows are still visible behind the lock"
    assert locked.upper() == "#1E1E2E", "lock screen should show the palette background"
    assert bar.send("workspace 1 1") == "v1 error invalid-command", "commands must be refused while locked"
    assert comp.stop(), "socket not removed on exit"
    print("session lock OK")
    print("OK")
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-3000:])
    raise
