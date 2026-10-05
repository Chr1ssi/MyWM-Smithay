"""The screen-cast portal (mywm-portal) ends with its session bus instead of lingering as an orphan.

A session bus that goes away (a test bus, a session without systemd) does not stop the services it
activated. Needs dbus-daemon, busctl and `cargo build --workspace`. Run from the repository root:
    PYTHONPATH=tests python3 tests/portal_bus_smoke.py
"""
import os, subprocess, tempfile
from smoke_support import private_bus, wait

work = tempfile.mkdtemp()
bus_command, address = private_bus(work)
env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=address)
bus = subprocess.Popen(bus_command, env=env)
portal = None
try:
    assert wait(lambda: os.path.exists(f"{work}/bus")), "no bus"
    portal = subprocess.Popen(["./target/debug/mywm-portal"], env=env, stdout=open(f"{work}/portal.log", "w"), stderr=subprocess.STDOUT)

    def owned():
        names = subprocess.run(["busctl", f"--address={address}", "list", "--no-legend"], env=env, capture_output=True, text=True).stdout
        return "org.freedesktop.impl.portal.desktop.mywm" in names
    assert wait(owned), "the portal did not take its bus name"

    bus.terminate()
    bus.wait(5)
    assert wait(lambda: portal.poll() is not None, 10), "the portal outlived its bus"
    assert portal.returncode == 0, portal.returncode
    print("OK")
except BaseException:
    print(open(f"{work}/portal.log").read()[-2000:] if os.path.exists(f"{work}/portal.log") else "no portal log")
    raise
finally:
    for process in (portal, bus):
        if process is not None and process.poll() is None:
            process.kill()
