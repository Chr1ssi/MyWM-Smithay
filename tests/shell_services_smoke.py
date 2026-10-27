"""The built-in shell's services process (splice-shell-services) against a private PipeWire with a null sink, a stand-in
media player and a stand-in for BlueZ: the wheel over the audio button changes the volume and a right click mutes,
the playing title shows in the bar's middle, Bluetooth is followed, and a killed services process comes back.

Needs dbus-daemon, pipewire, wireplumber, pw-cli, wpctl, busctl, python3 with dbus and gi, and
`cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/shell_services_smoke.py
"""
import os, re, shutil, subprocess, sys, tempfile, time
from smoke_support import Bar, Compositor, hermetic_wireplumber, private_bus, wait

for tool in ("dbus-daemon", "pipewire", "wireplumber", "pw-cli", "wpctl", "busctl", "grim"):
    if not shutil.which(tool):
        print(f"SKIP: {tool} is not installed")
        sys.exit(0)
try:
    import dbus, gi  # noqa: F401  (the stand-ins need them)
except ImportError:
    print("SKIP: python3 without dbus and gi")
    sys.exit(0)

BACKGROUND = "#1E1E2E"
SURFACE = "#313244"
# On the audio button whatever it says ("󰕾 —" or "󰕾 50 %"): left of the power island.
AUDIO = (1200, 22)

work = tempfile.mkdtemp()
os.makedirs(f"{work}/session")
os.makedirs(f"{work}/system")
processes = []
session_bus, session_address = private_bus(f"{work}/session")
system_bus, system_address = private_bus(f"{work}/system")
env = dict(os.environ, PIPEWIRE_RUNTIME_DIR=work, DBUS_SESSION_BUS_ADDRESS=session_address,
           DBUS_SYSTEM_BUS_ADDRESS=system_address, XDG_STATE_HOME=f"{work}/state")


def spawn(args, log, extra=None):
    process = subprocess.Popen(args, env=dict(env, **(extra or {})), stdout=open(f"{work}/{log}.log", "w"), stderr=subprocess.STDOUT)
    processes.append(process)
    return process


def volume():
    out = subprocess.run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], env=env, capture_output=True, text=True).stdout
    found = re.search(r"Volume: ([0-9.]+)( \[MUTED\])?", out)
    return (round(float(found.group(1)), 2), bool(found.group(2))) if found else None


def pixels(comp, x, y, w, h):
    shot = subprocess.run(["grim", "-g", f"{x},{y} {w}x{h}", "-t", "ppm", "-"], env=comp.env(),
                          capture_output=True, check=True, timeout=15).stdout
    return shot[-w * h * 3:]


def background(data):
    color = bytes.fromhex(BACKGROUND[1:])
    return all(data[i:i + 3] == color for i in range(0, len(data), 3))


def mouse(*args):
    subprocess.run(["xdotool", "mousemove", str(AUDIO[0]), str(AUDIO[1]), *args], check=True)


def reported(comp, pattern):
    return re.search(pattern, open(comp.dir + "/log").read())


comp = None
try:
    spawn(session_bus, "session-bus")
    spawn(system_bus, "system-bus")
    assert wait(lambda: os.path.exists(f"{work}/session/bus") and os.path.exists(f"{work}/system/bus")), "no buses"
    spawn(["pipewire"], "pipewire")
    assert wait(lambda: os.path.exists(f"{work}/pipewire-0")), "no pipewire"
    spawn(["wireplumber"], "wireplumber", hermetic_wireplumber(f"{work}/wireplumber-config"))
    time.sleep(2)
    subprocess.run(["pw-cli", "create-node", "adapter",
                    "{ factory.name=support.null-audio-sink node.name=test-sink media.class=Audio/Sink "
                    "audio.channels=2 audio.position=[FL FR] object.linger=true }"], env=env, check=True, capture_output=True)
    assert wait(lambda: volume() is not None, 10), "the null sink did not become the default"
    subprocess.run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.5"], env=env, check=True)
    spawn(["python3", "tests/fake_bluez.py", f"{work}/bluez.calls"], "bluez")
    spawn(["python3", "tests/fake_mpris.py", "Testlied"], "mpris")

    extra = {name: env[name] for name in ("PIPEWIRE_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS")}
    extra["RUST_LOG"] = "info,splice_compositor::shell_services=debug"
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    bar = Bar(comp.sock)
    bar.until(lambda s: len(s) == 1)
    assert wait(lambda: pixels(comp, 15, 22, 1, 1) == bytes.fromhex(SURFACE[1:]), 20), "the bar was not drawn"
    assert wait(lambda: reported(comp, r"audio: Some\(Audio \{ volume: 0\.5"), 15), "no volume from the services"
    print("services report the volume OK")

    # The wheel: 5 % a notch, up and down; a right click mutes and unmutes. Nested, winit reports both the press
    # and the release of an X wheel button as a notch, so one click here is two notches.
    mouse("click", "4")
    assert wait(lambda: volume() == (0.6, False)), volume()
    mouse("click", "5", "click", "5")
    assert wait(lambda: volume() == (0.4, False)), volume()
    mouse("click", "3")
    assert wait(lambda: volume() == (0.4, True)), volume()
    mouse("click", "3")
    assert wait(lambda: volume() == (0.4, False)), volume()
    print("wheel and right click on the audio button OK")

    # The playing title in the middle, left of the clock; paused, it goes.
    title = (500, 12, 40, 20)
    assert wait(lambda: not background(pixels(comp, *title)), 10), "the playing title did not show"
    subprocess.run(["busctl", f"--address={session_address}", "call", "org.mpris.MediaPlayer2.splicetest", "/org/mpris/MediaPlayer2",
                    "org.splice.Test", "SetPlaying", "b", "false"], env=env, check=True)
    assert wait(lambda: background(pixels(comp, *title)), 10), "the paused player still shows"
    print("media title OK")

    # Bluetooth: switched on, then a device connects.
    assert reported(comp, r"bluetooth: Some\(Bluetooth \{ powered: false"), "no adapter reported"
    busctl = ["busctl", f"--address={system_address}", "call", "org.bluez"]
    subprocess.run([*busctl, "/org/bluez/hci0", "org.freedesktop.DBus.Properties", "Set", "ssv", "org.bluez.Adapter1", "Powered", "b", "true"],
                   env=env, check=True)
    assert wait(lambda: reported(comp, r"bluetooth: Some\(Bluetooth \{ powered: true"), 10), "the adapter's power was not followed"
    subprocess.run([*busctl, "/org/bluez/hci0/dev_11_11_11_11_11_11", "org.bluez.Device1", "Connect"], env=env, check=True)
    connected = r'name: "Kopfhörer", address: "[^"]*", paired: true, connected: true'
    assert wait(lambda: reported(comp, connected), 10), "the connected device was not followed"
    print("Bluetooth OK")

    # Killed, the services process comes back, and the compositor stays responsive meanwhile.
    pids = lambda: subprocess.run(["pgrep", "-P", str(comp.process.pid), "-x", "splice-shell-se"], capture_output=True, text=True).stdout.split()
    first = pids()
    assert len(first) == 1, first
    subprocess.run(["kill", "-9", first[0]], check=True)
    assert bar.send(f"workspace {bar.state[0]['id']} 1") == "v1 ok"
    assert wait(lambda: pids() and pids() != first, 10), "the services process was not started again"
    time.sleep(2)
    mouse("click", "4")
    assert wait(lambda: volume() == (0.5, False)), volume()
    print("services restart OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-4000:])
    for name in ("pipewire", "wireplumber", "bluez", "mpris"):
        if os.path.exists(f"{work}/{name}.log"):
            print(f"--- {name}\n" + open(f"{work}/{name}.log").read()[-1500:])
    raise
finally:
    for process in processes:
        process.kill()
