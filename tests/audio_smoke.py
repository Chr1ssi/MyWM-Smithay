"""The built-in shell's audio panel against a private PipeWire: the default output with its slider (dragging sets the
volume) and level button (which mutes), the button that shows the other outputs, choosing one of them as the
default, and the lists without WirePlumber's smart filter but with the application that plays.

Needs dbus-daemon, pipewire, wireplumber, pw-cli, wpctl and `cargo build --workspace`. Run from the repository root with
$DISPLAY set:
    PYTHONPATH=tests python3 tests/audio_smoke.py
"""
import os, re, shutil, subprocess, sys, tempfile, time
from smoke_support import Compositor, hermetic_wireplumber, private_bus, wait

for tool in ("dbus-daemon", "pipewire", "wireplumber", "pw-cli", "wpctl", "grim"):
    if not shutil.which(tool):
        print(f"SKIP: {tool} is not installed")
        sys.exit(0)

AUDIO_BUTTON = (1200, 22)
PANEL_X = 1280 - 8 - 400 + 20  # the panel's content
SLIDER_X = PANEL_X + 12  # the slider, inside the device's card; its track is 336 wide
# Middles of the targets on 1280x800 with DejaVu Sans (the panel's title and headings set the heights): the default
# output's level button, its slider, the button to show the other outputs and, once shown, the first of them.
ROW = 149
SLIDER = 182
EXPAND = 222
OTHER = 265

work = tempfile.mkdtemp()
os.makedirs(f"{work}/session")
processes = []
bus_command, address = private_bus(f"{work}/session")
env = dict(os.environ, PIPEWIRE_RUNTIME_DIR=work, DBUS_SESSION_BUS_ADDRESS=address, XDG_STATE_HOME=f"{work}/state")


def spawn(args, log, extra=None):
    process = subprocess.Popen(args, env=dict(env, **(extra or {})), stdout=open(f"{work}/{log}.log", "w"), stderr=subprocess.STDOUT)
    processes.append(process)
    return process


def node(*properties):
    subprocess.run(["pw-cli", "create-node", "adapter", "{ factory.name=support.null-audio-sink object.linger=true " + " ".join(properties) + " }"],
                   env=env, check=True, capture_output=True)


def wpctl(*args):
    return subprocess.run(["wpctl", *args], env=env, capture_output=True, text=True).stdout


def volume():
    found = re.search(r"Volume: ([0-9.]+)( \[MUTED\])?", wpctl("get-volume", "@DEFAULT_AUDIO_SINK@"))
    return (float(found.group(1)), bool(found.group(2))) if found else None


def default_sink():
    found = re.search(r'node.name = "([^"]+)"', wpctl("inspect", "@DEFAULT_AUDIO_SINK@"))
    return found.group(1) if found else None


def pixel(comp, x, y):
    shot = subprocess.run(["grim", "-g", f"{x},{y} 1x1", "-t", "ppm", "-"], env=comp.env(), capture_output=True, check=True, timeout=15).stdout
    return shot[-3:]


def mouse(*args):
    subprocess.run(["xdotool", *args], check=True)


comp = None
try:
    spawn(bus_command, "bus")
    assert wait(lambda: os.path.exists(f"{work}/session/bus")), "no bus"
    spawn(["pipewire"], "pipewire")
    assert wait(lambda: os.path.exists(f"{work}/pipewire-0")), "no pipewire"
    spawn(["wireplumber"], "wireplumber", hermetic_wireplumber(f"{work}/wireplumber-config"))
    time.sleep(2)
    for name in ("test-sink", "test-sink-2"):
        node(f"node.name={name}", "media.class=Audio/Sink", "audio.position=[FL FR]")
    node("node.name=test-mic", "media.class=Audio/Source/Virtual", "audio.position=[FL FR]")
    node("node.name=test-mic-filter", "media.class=Audio/Source", "filter.smart=true", "audio.position=[FL FR]")
    node("node.name=test-stream", "media.class=Stream/Output/Audio", "application.name=Testapp", "audio.position=[FL FR]")
    assert wait(lambda: default_sink() is not None, 10), "no default output"
    first = "test-sink"
    wpctl("set-default", re.search(r"id (\d+)", wpctl("inspect", "@DEFAULT_AUDIO_SINK@")).group(1))
    subprocess.run(["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.5"], env=env, check=True)
    first = default_sink()

    extra = {name: env[name] for name in ("PIPEWIRE_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS")}
    extra.update(DBUS_SYSTEM_BUS_ADDRESS="unix:path=/nonexistent", RUST_LOG="info,mywm_compositor::shell_services=debug", LANG="de_DE.UTF-8")
    comp = Compositor(top="builtin_shell = true\n", extra_env=extra)
    log = lambda: open(comp.dir + "/log").read()
    assert wait(lambda: re.search(r'application: "Testapp"', log()), 20), "no application stream reported"
    status = log().rsplit("shell services: ", 1)[-1]
    assert 'name: "test-mic"' in status and 'test-mic-filter' not in status, status[:2000]
    print("nodes reported, smart filter left out OK")

    # The fonts load in the background; the bar takes clicks once it is drawn (the selected workspace's pill).
    assert wait(lambda: pixel(comp, 15, 22) == bytes.fromhex("313244"), 20), "the bar was not drawn"
    mouse("mousemove", *map(str, AUDIO_BUTTON), "click", "1")
    time.sleep(1)
    # Drag the default output's slider from a quarter to three quarters.
    quarter, three_quarters = SLIDER_X + 7 + 0.25 * 322, SLIDER_X + 7 + 0.75 * 322
    mouse("mousemove", str(int(quarter)), str(SLIDER), "mousedown", "1")
    assert wait(lambda: volume() and abs(volume()[0] - 0.25) < 0.02, 5), volume()
    mouse("mousemove", str(int(three_quarters)), str(SLIDER), "mouseup", "1")
    assert wait(lambda: volume() and abs(volume()[0] - 0.75) < 0.02, 5), volume()
    print("slider OK")
    # The level button mutes.
    mouse("mousemove", str(PANEL_X + 360 - 8 - 39), str(ROW), "click", "1")
    assert wait(lambda: volume() and volume()[1], 5), volume()
    print("mute OK")
    # Another output, chosen as the default; the list folds up again.
    mouse("mousemove", str(PANEL_X + 180), str(EXPAND), "click", "1")
    time.sleep(0.5)
    mouse("mousemove", str(PANEL_X + 100), str(OTHER), "click", "1")
    assert wait(lambda: default_sink() not in (None, first), 5), default_sink()
    print("default output chosen OK")
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-3000:])
    raise
finally:
    for process in processes:
        process.kill()
