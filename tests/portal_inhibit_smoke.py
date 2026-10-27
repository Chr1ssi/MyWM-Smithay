"""Apps that keep the session awake through the portal (Firefox does) reach the compositor's idle inhibition.

Runs splice-portal on a private bus and calls its Inhibit backend the way xdg-desktop-portal does.
Needs swayidle, dbus-daemon, busctl and `cargo build --workspace`. Run from the repository root with
$DISPLAY set: PYTHONPATH=tests python3 tests/portal_inhibit_smoke.py
"""
import os, subprocess, tempfile, time
from smoke_support import Compositor, private_bus, wait

work = tempfile.mkdtemp()
IDLE = f"{work}/idle"
NAME = "org.freedesktop.impl.portal.desktop.splice"
comp = Compositor(extra_args=f"swayidle -w timeout 2 'touch {IDLE}' & ")
bus_command, address = private_bus(work)
env = dict(comp.env(), DBUS_SESSION_BUS_ADDRESS=address)
bus = subprocess.Popen(bus_command, env=env)
portal = None


def busctl(*args):
    subprocess.run(["busctl", f"--address={address}", *args], env=env, check=True, capture_output=True)


def inhibit(handle):
    busctl("call", NAME, "/org/freedesktop/portal/desktop", "org.freedesktop.impl.portal.Inhibit", "Inhibit",
           "ossua{sv}", handle, "org.mozilla.firefox", "", "8", "0")


def stays_awake(seconds):
    """Input first: swayidle reports idleness only once until something happens."""
    subprocess.run(["xdotool", "mousemove", "300", "300"], check=True)
    subprocess.run(["xdotool", "mousemove", "310", "310"], check=True)
    time.sleep(0.3)
    if os.path.exists(IDLE):
        os.remove(IDLE)
    return not wait(lambda: os.path.exists(IDLE), seconds)


def start_portal():
    process = subprocess.Popen(["./target/debug/splice-portal"], env=env, stdout=open(f"{work}/portal.log", "a"), stderr=subprocess.STDOUT)
    def owned():
        names = subprocess.run(["busctl", f"--address={address}", "list", "--no-legend"], env=env, capture_output=True, text=True).stdout
        return NAME in names
    assert wait(owned), "the portal did not take its bus name"
    return process


try:
    assert wait(lambda: os.path.exists(f"{work}/bus")), "no bus"
    assert wait(lambda: os.path.exists(IDLE), 8), "swayidle never saw the session go idle"
    portal = start_portal()

    handle = "/org/freedesktop/portal/desktop/request/1_7/t1"
    inhibit(handle)
    assert stays_awake(5), "went idle although an app inhibits through the portal"
    print("portal inhibition keeps the session awake OK")
    busctl("call", NAME, handle, "org.freedesktop.impl.portal.Request", "Close")
    assert not stays_awake(5), "still awake after the app closed its inhibition"
    print("closing the request ends it OK")

    # A portal that dies takes its inhibitions along: its connection to the compositor closes.
    inhibit("/org/freedesktop/portal/desktop/request/1_7/t2")
    assert stays_awake(4), "the second inhibition did not take"
    portal.kill()
    portal.wait()
    assert not stays_awake(5), "an inhibition outlived the portal"
    print("a crashed portal releases its inhibitions OK")
    assert comp.stop()
except BaseException:
    comp.process.kill()
    print(open(comp.dir + "/log").read()[-2000:])
    print(open(f"{work}/portal.log").read()[-2000:] if os.path.exists(f"{work}/portal.log") else "no portal log")
    raise
finally:
    for process in (portal, bus):
        if process is not None and process.poll() is None:
            process.kill()
print("OK")
