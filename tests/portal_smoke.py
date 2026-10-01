"""The screen-cast portal (mywm-portal): choose a source in the compositor, stream it over PipeWire.

Needs dbus-daemon, pipewire, wireplumber, busctl, gst-launch-1.0 with the pipewire plugin, and
`cargo build --workspace`. Run from the repository root with $DISPLAY set:
    PYTHONPATH=tests python3 tests/portal_smoke.py
"""
import os, re, shutil, subprocess, sys, tempfile, time
from smoke_support import Compositor, wait

for tool in ("dbus-daemon", "pipewire", "wireplumber", "busctl", "gst-launch-1.0"):
    if not shutil.which(tool):
        print(f"SKIP: {tool} is not installed")
        sys.exit(0)

work = tempfile.mkdtemp()
processes = []
bus_address = f"unix:path={work}/bus"
pw_env = dict(os.environ, PIPEWIRE_RUNTIME_DIR=work, DBUS_SESSION_BUS_ADDRESS=bus_address)


def spawn(args, env, log):
    process = subprocess.Popen(args, env=env, stdout=open(f"{work}/{log}.log", "w"), stderr=subprocess.STDOUT)
    processes.append(process)
    return process


def busctl(*args, timeout=30):
    return subprocess.run(["busctl", f"--address={bus_address}", "--user", "call", *args], env=pw_env, capture_output=True, text=True, timeout=timeout)


PORTAL = ["org.freedesktop.impl.portal.desktop.mywm", "/org/freedesktop/portal/desktop", "org.freedesktop.impl.portal.ScreenCast"]
def brightness(path):
    out = subprocess.check_output(["convert", path, "-resize", "1x1!", "-colorspace", "Gray", "-format", "%[fx:mean]", "info:"])
    return float(out.decode())


comp = None
try:
    spawn(["dbus-daemon", "--session", "--nofork", f"--address={bus_address}"], pw_env, "dbus")
    assert wait(lambda: os.path.exists(f"{work}/bus")), "no bus"
    spawn(["pipewire"], pw_env, "pipewire")
    assert wait(lambda: os.path.exists(f"{work}/pipewire-0")), "no pipewire"
    # WirePlumber remembers the target of a consumer (restore-stream) and would send the next
    # gst-launch to the node of the previous stream, so every stream gets a fresh one.
    wireplumber = []

    def fresh_wireplumber():
        for old in wireplumber:
            old.kill()
            old.wait()
        state = tempfile.mkdtemp(dir=work)
        wireplumber[:] = [spawn(["wireplumber"], dict(pw_env, XDG_STATE_HOME=state), f"wireplumber{len(processes)}")]
        time.sleep(2)

    fresh_wireplumber()

    comp = Compositor(windows=1)
    time.sleep(3)
    portal_env = dict(pw_env, WAYLAND_DISPLAY=comp.display, MYWM_SOCKET=comp.sock, RUST_LOG="info")
    spawn(["./target/debug/mywm-portal"], portal_env, "portal")
    assert wait(lambda: busctl(*PORTAL[:1], "/", "org.freedesktop.DBus.Peer", "Ping").returncode == 0, 10), open(f"{work}/portal.log").read()

    win = subprocess.check_output(["xdotool", "search", "--onlyvisible", "--name", "Smithay"]).split()[0].decode()

    def stream(choose, label):
        fresh_wireplumber()
        session = f"/org/freedesktop/portal/desktop/session/test/{label}"
        r = busctl(*PORTAL[:2], PORTAL[2], "CreateSession", "oosa{sv}", f"/req/{label}1", session, f"app-{label}", "0")
        assert r.returncode == 0 and r.stdout.startswith("ua{sv} 0"), r.stderr + r.stdout
        r = busctl(*PORTAL[:2], PORTAL[2], "SelectSources", "oosa{sv}", f"/req/{label}2", session, f"app-{label}", "2", "types", "u", "3", "cursor_mode", "u", "2")
        assert r.returncode == 0 and r.stdout.startswith("ua{sv} 0"), r.stderr + r.stdout
        start = subprocess.Popen(["busctl", f"--address={bus_address}", "--user", "call", *PORTAL, "Start", "oossa{sv}", f"/req/{label}3", session, f"app-{label}", "", "0"],
                                 env=pw_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        time.sleep(1.5)
        choose()
        out, err = start.communicate(timeout=30)
        assert start.returncode == 0, err + out + open(f"{work}/portal.log").read()
        match = re.search(r'a\(ua\{sv\}\) 1 (\d+) \d+ ".*', out)
        assert match, out
        node = int(match.group(1))
        size = re.search(r'"size" \(ii\) (\d+) (\d+)', out).groups()
        source_type = re.search(r'"source_type" u (\d+)', out).group(1)
        return session, node, size, source_type

    def grab(node, label, caps=None):
        path = f"{work}/{label}.png"
        convert = ["!", caps] if caps else []
        gst = subprocess.run(
            ["gst-launch-1.0", "-q", "pipewiresrc", "target-object=mywm-screencast", "num-buffers=3", *convert, "!", "videoconvert", "!", "pngenc", "snapshot=true", "!", "filesink", f"location={path}"],
            env=pw_env, capture_output=True, text=True, timeout=30)
        assert gst.returncode == 0 and os.path.exists(path), gst.stderr + open(f"{work}/portal.log").read()
        return subprocess.check_output(["identify", "-format", "%wx%h", path]).decode(), path

    # A monitor: Enter in the chooser shares the monitor under the pointer.
    session, node, size, source_type = stream(lambda: comp.key("Return"), "monitor")
    assert source_type == "1" and tuple(size) == ("1280", "800"), (source_type, size)
    dims, path = grab(node, "monitor")
    assert dims == "1280x800", dims
    assert brightness(path) > 0.1, "the monitor stream is black"
    print("monitor stream:", dims)
    closed = busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")
    assert closed.returncode == 0, closed.stderr

    # A window: click it in the chooser.
    def click_window():
        for args in (["mousemove", "--window", win, "640", "400"], ["click", "1"]):
            subprocess.run(["xdotool", *args], check=True)
            time.sleep(0.2)

    session, node, size, source_type = stream(click_window, "window")
    assert source_type == "2", source_type
    dims, path = grab(node, "window")
    assert dims == "1260x780", dims
    assert brightness(path) > 0.1, "the window stream is black"
    print("window stream:", dims)
    closed = busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")
    assert closed.returncode == 0, closed.stderr

    # The window resizes (a second window tiles next to it): the stream follows with new caps.
    session, node, size, source_type = stream(click_window, "resize")
    watcher = subprocess.Popen(["gst-launch-1.0", "-v", "pipewiresrc", "target-object=mywm-screencast", "num-buffers=200", "!", "videoconvert", "!", "fakesink"],
                               env=pw_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    time.sleep(2)
    subprocess.Popen(["weston-simple-shm"], env=comp.env(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(4)
    watcher.terminate()
    caps = watcher.communicate(timeout=10)[0]
    widths = re.findall(r"width=\(int\)(\d+)", caps)
    assert "1260" in widths and any(int(w) < 1000 for w in widths), ("stream did not follow the resize", widths, caps[-600:])
    print("resize followed:", sorted(set(widths)))
    closed = busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")
    assert closed.returncode == 0, closed.stderr

    # A consumer that asks for BGRA (Chromium does) must get opaque pixels.
    session, node, size, source_type = stream(lambda: comp.key("Return"), "bgra")
    dims, path = grab(node, "bgra", caps="video/x-raw,format=BGRA")
    opaque = subprocess.check_output(["identify", "-format", "%[opaque]", path]).decode()
    assert opaque.lower() == "true", f"BGRA frames are not opaque: {opaque}"
    print("BGRA opaque OK")
    closed = busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")
    assert closed.returncode == 0, closed.stderr

    # A still picture keeps delivering frames (keepalive): no window is animating any more.
    subprocess.run(["pkill", "-f", "^weston-simple-shm"])
    time.sleep(1)
    session, node, size, source_type = stream(lambda: comp.key("Return"), "still")
    start = time.time()
    # Four frames of a picture that never changes: the first at once, the others from the keepalive.
    gst = subprocess.run(["gst-launch-1.0", "-q", "pipewiresrc", "target-object=mywm-screencast", "num-buffers=4", "!", "videoconvert", "!", "fakesink"],
                         env=pw_env, capture_output=True, text=True, timeout=30)
    assert gst.returncode == 0, gst.stderr
    took = time.time() - start
    assert 0.8 < took < 15, f"a still screen should deliver a frame about every 0.4 s, four took {took:.1f} s"
    print("still picture stream OK in %.1f s" % took)
    closed = busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")
    assert closed.returncode == 0, closed.stderr

    # Two sessions of one app at once (Vesktop does this): both succeed and share the choice.
    fresh_wireplumber()
    sessions = []
    for label in ("a", "b"):
        session = f"/org/freedesktop/portal/desktop/session/test/two{label}"
        busctl(*PORTAL[:2], PORTAL[2], "CreateSession", "oosa{sv}", f"/req/two{label}1", session, "test", "0")
        busctl(*PORTAL[:2], PORTAL[2], "SelectSources", "oosa{sv}", f"/req/two{label}2", session, "test", "2", "types", "u", "3")
        sessions.append(session)
    starts = []
    for index, session in enumerate(sessions):
        starts.append(subprocess.Popen(["busctl", f"--address={bus_address}", "--user", "call", *PORTAL, "Start", "oossa{sv}", f"/req/two{index}3", session, "test", "", "0"],
                                       env=pw_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
        time.sleep(0.4)
    time.sleep(1)
    comp.key("Return")
    outputs = [p.communicate(timeout=30) for p in starts]
    assert all(p.returncode == 0 and out.startswith("ua{sv} 0") for p, (out, _) in zip(starts, outputs)), outputs
    print("two sessions at once OK")
    for session in sessions:
        busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")

    # Vesktop's second question within a moment reuses the first answer; a later one asks again.
    def ask(label):
        session = f"/org/freedesktop/portal/desktop/session/test/{label}"
        busctl(*PORTAL[:2], PORTAL[2], "CreateSession", "oosa{sv}", f"/req/{label}1", session, "again", "0")
        busctl(*PORTAL[:2], PORTAL[2], "SelectSources", "oosa{sv}", f"/req/{label}2", session, "again", "2", "types", "u", "3")
        return session, subprocess.Popen(["busctl", f"--address={bus_address}", "--user", "call", *PORTAL, "Start", "oossa{sv}", f"/req/{label}3", session, "again", "", "0"],
                                         env=pw_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    fresh_wireplumber()
    first, p1 = ask("reuse1")
    time.sleep(1.5)
    comp.key("Return")
    out, _ = p1.communicate(timeout=30)
    assert out.startswith("ua{sv} 0"), out
    second, p2 = ask("reuse2")
    out, _ = p2.communicate(timeout=10)  # no key press: the answer is reused
    assert out.startswith("ua{sv} 0"), out
    print("a follow-up session reuses the choice")
    for session in (first, second):
        busctl("org.freedesktop.impl.portal.desktop.mywm", session, "org.freedesktop.impl.portal.Session", "Close")
    time.sleep(9)
    third, p3 = ask("reuse3")
    time.sleep(1.5)
    assert p3.poll() is None, "after the window the chooser must ask again"
    comp.key("Return")
    out, _ = p3.communicate(timeout=30)
    assert out.startswith("ua{sv} 0"), out
    busctl("org.freedesktop.impl.portal.desktop.mywm", third, "org.freedesktop.impl.portal.Session", "Close")

    # Escape cancels the chooser: response 1.
    def cancel():
        comp.key("Escape")

    session = "/org/freedesktop/portal/desktop/session/test/cancel"
    busctl(*PORTAL[:2], PORTAL[2], "CreateSession", "oosa{sv}", "/req/c1", session, "test", "0")
    busctl(*PORTAL[:2], PORTAL[2], "SelectSources", "oosa{sv}", "/req/c2", session, "test", "1", "types", "u", "3")
    start = subprocess.Popen(["busctl", f"--address={bus_address}", "--user", "call", *PORTAL, "Start", "oossa{sv}", "/req/c3", session, "test", "", "0"],
                             env=pw_env, stdout=subprocess.PIPE, text=True)
    time.sleep(1.5)
    cancel()
    out, _ = start.communicate(timeout=20)
    assert out.startswith("ua{sv} 1"), out
    assert comp.stop()
    print("OK")
except BaseException:
    if comp:
        comp.process.kill()
        print(open(comp.dir + "/log").read()[-2000:])
    print("exit codes:", [(p.args[0], p.poll()) for p in processes])
    for name in ("portal", "pipewire", "dbus"):
        if os.path.exists(f"{work}/{name}.log"):
            print(f"--- {name}\n" + open(f"{work}/{name}.log").read()[-2500:])
    raise
finally:
    for process in processes:
        process.kill()
