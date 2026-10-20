"""Helpers for the smoke tests: run the nested compositor and talk to its bar socket.

Needs an X server ($DISPLAY, e.g. Xvfb :99) and a debug build of the workspace (`cargo build --workspace`:
the compositor and mywm-test-client). `tests/run-smoke` sets all of this up.
"""
import os, socket, subprocess, tempfile, time

DEBUG_BINARY = "./target/debug/mywm-compositor"
# An ordinary animated window (the role weston-simple-shm used to play).
WINDOW_CLIENT = "./target/debug/mywm-test-client simple 3600"


def private_bus(directory):
    """`dbus-daemon` arguments for a session bus at `<directory>/bus` that activates nothing: the
    default session config also reads the system's service files and would start the installed
    portals (with the real session's environment) instead of the ones under test."""
    config = f"{directory}/bus.conf"
    with open(config, "w") as f:
        f.write("""<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=%s/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
""" % directory)
    return ["dbus-daemon", "--nofork", f"--config-file={config}"], f"unix:path={directory}/bus"


def hermetic_wireplumber(directory):
    """Environment for a private WirePlumber that leaves the machine's sound, Bluetooth and camera hardware alone:
    without this, its monitors find the real ALSA devices, and a test that sets the default output's volume could
    set a real device's."""
    conf = f"{directory}/wireplumber/wireplumber.conf.d"
    os.makedirs(conf, exist_ok=True)
    with open(f"{conf}/90-no-hardware.conf", "w") as f:
        f.write("wireplumber.profiles = {\n  main = {\n    hardware.audio = disabled\n"
                "    hardware.bluetooth = disabled\n    hardware.video-capture = disabled\n  }\n}\n")
    return {"XDG_CONFIG_HOME": directory}


def wait(cond, timeout=8):
    end = time.time() + timeout
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.05)
    return False


class Compositor:
    def __init__(self, config="", extra_env=None, windows=0, extra_args="", top="", layout="us"):
        self.dir = tempfile.mkdtemp()
        self.sock = f"{self.dir}/ctl.sock"
        # An empty wallpaper directory, so the compositor does not draw one from the user's pictures.
        os.makedirs(f"{self.dir}/wallpapers")
        if "wallpaper_directory" not in top:
            top = f'wallpaper_directory = "{self.dir}/wallpapers"\n' + top
        with open(f"{self.dir}/config.toml", "w") as f:
            # `top` holds top-level keys, which must precede the first table.
            f.write(top + f'[keyboard]\nlayout = "{layout}"\n' + config)
        # Isolated from the user's session: own state directory (theme, wallpaper), no log file (the compositor
        # would rotate the real session's log), and no environment of a running mywm session.
        os.makedirs(f"{self.dir}/state")
        env = dict(os.environ, MYWM_SOCKET=self.sock, MYWM_CONFIG=f"{self.dir}/config.toml", RUST_LOG="info",
                   XDG_STATE_HOME=f"{self.dir}/state", MYWM_LOG_FILE="off")
        for name in [name for name in env if name.startswith("MYWM_") and name not in ("MYWM_SOCKET", "MYWM_CONFIG")]:
            del env[name]
        env.update(extra_env or {})
        env.pop("WAYLAND_DISPLAY", None)
        clients = "sleep 1; " + extra_args + "".join(f"{WINDOW_CLIENT} & sleep 0.3; " for _ in range(windows)) + "wait"
        # Start with the X pointer in a corner where no window will be: a pointer left over
        # from an earlier run would otherwise move focus by hovering over a new window.
        subprocess.run(["xdotool", "mousemove", "1279", "799"], check=False)
        self.log = open(f"{self.dir}/log", "w")
        self.process = subprocess.Popen([DEBUG_BINARY, clients], env=env, stdout=self.log, stderr=subprocess.STDOUT)
        assert wait(lambda: os.path.exists(self.sock)), "bar socket missing"
        # The window clients start one after the other; tests count on all of them being there.
        assert wait(lambda: open(f"{self.dir}/log").read().count("new window") >= windows, 30), "the windows did not appear"

    @property
    def display(self):
        """Name of the compositor's Wayland socket (from its log)."""
        import re
        assert wait(lambda: re.search(r'WAYLAND_DISPLAY="([^"]+)"', open(self.dir + "/log").read()))
        return re.search(r'WAYLAND_DISPLAY="([^"]+)"', open(self.dir + "/log").read()).group(1)

    def env(self):
        """Environment for running a Wayland client against this compositor."""
        env = dict(os.environ, WAYLAND_DISPLAY=self.display, MYWM_SOCKET=self.sock, MYWM_CONFIG=self.dir + "/config.toml")
        return env

    def stop(self):
        self.process.terminate()
        self.process.wait(5)
        return not os.path.exists(self.sock)

    def key(self, keys):
        """Send a key combination such as 'super+shift+Right' to the nested window."""
        window = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()
        subprocess.run(["xdotool", "windowfocus", window], check=True)
        subprocess.run(["xdotool", "key", "--delay", "80", keys], check=True)


class Bar:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.connect(path)
        self.socket.settimeout(0.1)
        self.pending = b""
        self.state = None
        self.locked = False
        self.gaming = False
        self.config_error = None
        self.capturing = False
        self.scratchpad = (False, False)  # visible, occupied
        self.picking = None  # None, or the color under the pointer ("" until known) while the color picker is on
        self.replies = []
        self.confirm_logouts = []

    def poll(self):
        try:
            self.pending += self.socket.recv(65536)
        except socket.timeout:
            pass
        *lines, self.pending = self.pending.split(b"\n")
        for line in lines:
            text = line.decode()
            if text.startswith("v1 state"):
                self.state = parse_state(text)
            elif text.startswith("v1 picking "):
                fields = text.split()
                self.picking = None if fields[2] == "0" else (fields[3] if len(fields) > 3 else "")
            elif text.startswith("v1 scratchpad "):
                self.scratchpad = tuple(flag == "1" for flag in text.split()[2:4])
            elif text.startswith("v1 locked"):
                self.locked = text.endswith("1")
            elif text.startswith("v1 gaming"):
                self.gaming = text.endswith("1")
            elif text.startswith("v1 capture "):
                self.capturing = text.endswith("1")
            elif text.startswith("v1 config "):
                self.config_error = None if text == "v1 config ok" else text.removeprefix("v1 config error ")
            elif text.startswith("v1 confirm-logout "):
                self.confirm_logouts.append(int(text.split()[2]))
            elif text.startswith("v1 ok") or text.startswith("v1 error"):
                self.replies.append(text)

    def until(self, predicate, timeout=8):
        def check():
            self.poll()
            return self.state is not None and predicate(self.state)
        assert wait(check, timeout), f"condition not reached; state: {self.state}"

    def send(self, command):
        before = len(self.replies)
        self.socket.sendall(f"v1 {command}\n".encode())
        assert wait(lambda: (self.poll(), len(self.replies) > before)[1]), "no reply"
        return self.replies[-1]


def parse_state(line):
    """`v1 state id,x,y,w,h,active,left,right,n:o|n:o;...` -> list of dicts."""
    outputs = []
    for entry in filter(None, line.split(" ", 2)[2].split(";")):
        f = entry.split(",")
        outputs.append(dict(
            id=int(f[0]), x=int(f[1]), y=int(f[2]), width=int(f[3]), height=int(f[4]), active=int(f[5]),
            left=f[6] == "1", right=f[7] == "1",
            workspaces={int(n): o == "1" for n, o in (w.split(":") for w in f[8].split("|"))},
        ))
    return outputs
