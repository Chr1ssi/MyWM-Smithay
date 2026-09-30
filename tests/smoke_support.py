"""Helpers for the smoke tests: run the nested compositor and talk to its bar socket.

Needs an X server ($DISPLAY, e.g. Xvfb :99), weston-simple-shm and a debug build.
"""
import os, socket, subprocess, tempfile, time

DEBUG_BINARY = "./target/debug/mywm-compositor"


def wait(cond, timeout=8):
    end = time.time() + timeout
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.05)
    return False


class Compositor:
    def __init__(self, config="", extra_env=None, windows=0, extra_args=""):
        self.dir = tempfile.mkdtemp()
        self.sock = f"{self.dir}/ctl.sock"
        with open(f"{self.dir}/config.toml", "w") as f:
            f.write('[keyboard]\nlayout = "us"\n' + config)
        env = dict(os.environ, MYWM_SOCKET=self.sock, MYWM_CONFIG=f"{self.dir}/config.toml", RUST_LOG="info")
        env.update(extra_env or {})
        env.pop("WAYLAND_DISPLAY", None)
        clients = "sleep 1; " + extra_args + "".join("weston-simple-shm & sleep 0.3; " for _ in range(windows)) + "wait"
        # Start with the X pointer in a corner where no window will be: a pointer left over
        # from an earlier run would otherwise move focus by hovering over a new window.
        subprocess.run(["xdotool", "mousemove", "1279", "799"], check=False)
        self.log = open(f"{self.dir}/log", "w")
        self.process = subprocess.Popen([DEBUG_BINARY, clients], env=env, stdout=self.log, stderr=subprocess.STDOUT)
        assert wait(lambda: os.path.exists(self.sock)), "bar socket missing"

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
        self.replies = []

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
            elif text.startswith("v1 locked"):
                self.locked = text.endswith("1")
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
