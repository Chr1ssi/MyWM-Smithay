"""The built-in shell's command center against a stand-in for BlueZ (tests/fake_bluez.py on a private system bus) and
stand-ins for gammastep, ip, wl-copy and mywm-settings: Bluetooth on, connect, search, pair (trusted first), copy the address, the
night light on and off and remembered, the settings app started from the panel, and a saved night light starts with
the session.

Needs dbus-daemon, python3 with dbus and gi, and `cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/control_smoke.py
"""
import os, shutil, subprocess, sys, tempfile, time
from smoke_support import Compositor, private_bus, wait

for tool in ("dbus-daemon", "grim"):
    if not shutil.which(tool):
        print(f"SKIP: {tool} is not installed")
        sys.exit(0)
try:
    import dbus, gi  # noqa: F401  (the stand-in needs them)
except ImportError:
    print("SKIP: python3 without dbus and gi")
    sys.exit(0)

# Where the panel's targets are on 1280x800 (each lies well inside its row or tile, so a line a pixel higher or
# lower in another font does not matter).
BLUETOOTH_TILE = (986, 143)
NIGHT_TILE = (1171, 143)
ADDRESS_ROW = (1072, 241)
SEARCH_BUTTON = (1210, 290)
FIRST_DEVICE_BUTTON = (1200, 340)
SECOND_DEVICE_BUTTON = (1200, 400)
SETTINGS_ROW = (1072, 471)  # below the two devices
CONTROL_BUTTON = (1138, 22)  # the gear, left of the bell (the volume shows "—" here)

work = tempfile.mkdtemp()
stubs = f"{work}/bin"
os.makedirs(stubs)
record = f"{work}/record"
open(record, "w").close()


def stub(name, script):
    with open(f"{stubs}/{name}", "w") as f:
        f.write("#!/usr/bin/env bash\n" + script)
    os.chmod(f"{stubs}/{name}", 0o755)


# gammastep keeps running while the night light is on; ip reports one address; wl-copy takes it.
stub("gammastep", f'echo "gammastep $*" >> {record}\ntrap \'echo "gammastep stopped" >> {record}; exit 0\' TERM\nwhile true; do sleep 0.1; done\n')
stub("ip", 'echo "2: eno1    inet 192.168.1.23/24 brd 192.168.1.255 scope global eno1"\n')
stub("wl-copy", f'echo "copy $*" >> {record}\n')
stub("mywm-settings", f'echo settings >> {record}\n')

os.makedirs(f"{work}/system")
system_command, system_address = private_bus(f"{work}/system")
processes = [subprocess.Popen(system_command)]
assert wait(lambda: os.path.exists(f"{work}/system/bus")), "no system bus"
bluez_log = f"{work}/bluez"
open(bluez_log, "w").close()
processes.append(subprocess.Popen(["python3", "tests/fake_bluez.py", bluez_log], env=dict(os.environ, DBUS_SYSTEM_BUS_ADDRESS=system_address)))
assert wait(lambda: "ready" in open(bluez_log).read()), "the stand-in for BlueZ did not start"
state = f"{work}/state"
os.makedirs(f"{state}/mywm")


def calls():
    return open(bluez_log).read().splitlines()


def recorded():
    return open(record).read().splitlines()


def click(x, y):
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)
    time.sleep(0.4)


def compositor():
    extra = {"DBUS_SYSTEM_BUS_ADDRESS": system_address, "DBUS_SESSION_BUS_ADDRESS": "unix:path=/nonexistent",
             "PIPEWIRE_RUNTIME_DIR": "/nonexistent", "PATH": f"{stubs}:{os.environ['PATH']}", "XDG_STATE_HOME": state,
             "LANG": "de_DE.UTF-8"}
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    shot = lambda: subprocess.run(["grim", "-g", "15,22 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True).stdout[-3:]
    assert wait(lambda: shot() == bytes.fromhex("313244"), 20), "the bar was not drawn"
    return comp


comp = None
try:
    comp = compositor()
    click(*CONTROL_BUTTON)
    time.sleep(1)
    # Bluetooth is off: the tile switches it on, and the paired headset shows up.
    click(*BLUETOOTH_TILE)
    assert wait(lambda: "set hci0 Powered True" in calls()), calls()
    time.sleep(0.5)
    click(*FIRST_DEVICE_BUTTON)
    assert wait(lambda: "call dev_11_11_11_11_11_11 Connect" in calls()), calls()
    # A search finds the speaker; pairing trusts it too.
    click(*SEARCH_BUTTON)
    assert wait(lambda: "call hci0 StartDiscovery" in calls()), calls()
    time.sleep(0.5)
    click(*SECOND_DEVICE_BUTTON)
    assert wait(lambda: "call dev_22_22_22_22_22_22 Pair" in calls()), calls()
    assert "set dev_22_22_22_22_22_22 Trusted True" in calls(), calls()
    print("bluetooth OK")

    # The address row copies the address.
    click(*ADDRESS_ROW)
    assert wait(lambda: "copy 192.168.1.23" in recorded()), recorded()
    print("address OK")

    # The night light tile runs gammastep while on and remembers the choice.
    click(*NIGHT_TILE)
    assert wait(lambda: "gammastep -m wayland -O 4000" in recorded()), recorded()
    assert open(f"{state}/mywm/night-light").read().strip() == "1"
    click(*NIGHT_TILE)
    assert wait(lambda: "gammastep stopped" in recorded()), recorded()
    assert open(f"{state}/mywm/night-light").read().strip() == "0"
    # The settings row starts the settings app.
    click(*SETTINGS_ROW)
    assert wait(lambda: "settings" in recorded()), recorded()
    print("settings OK")
    # Saved as on, a new session starts it at once.
    with open(f"{state}/mywm/night-light", "w") as f:
        f.write("1\n")
    assert comp.stop()
    open(record, "w").close()
    comp = compositor()
    assert wait(lambda: "gammastep -m wayland -O 4000" in recorded()), recorded()
    print("night light OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    print("bluez:", calls())
    print("recorded:", recorded())
    raise
finally:
    for process in processes:
        process.kill()
